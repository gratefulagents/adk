// Interop-only executable, compiled inside an archived, pinned gratefulagents/sdk module.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"sync/atomic"
	"time"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	wrapper "github.com/gratefulagents/sdk/pkg/agentsdk/mcp"
	protocol "github.com/modelcontextprotocol/go-sdk/mcp"
)

const credential = "Bearer interop-test-only"
const tenant = "interop-tenant"
const uri = "test://interop/resource"

func check(err error) {
	if err != nil {
		panic(err)
	}
}
func emit(v any) { check(json.NewEncoder(os.Stdout).Encode(v)) }
func resource() *protocol.ReadResourceResult {
	return &protocol.ReadResourceResult{Contents: []*protocol.ResourceContents{{URI: uri, Text: "resource-value"}}}
}
func prompt() *protocol.GetPromptResult {
	return &protocol.GetPromptResult{Messages: []*protocol.PromptMessage{{Role: "user", Content: &protocol.TextContent{Text: "prompt-value"}}}}
}
func call(args json.RawMessage) string {
	var v struct {
		Value string `json:"value"`
	}
	check(json.Unmarshal(args, &v))
	return "echo:" + v.Value
}
func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 40*time.Second)
	defer cancel()
	if len(os.Args) < 2 {
		panic("mode required")
	}
	mode := os.Args[1]
	if mode == "stdio" && len(os.Args) == 3 {
		check(os.WriteFile(os.Args[2], []byte(fmt.Sprint(os.Getpid())), 0600))
	}
	if mode == "client" {
		client(ctx)
		return
	}
	var handler http.Handler
	var closeMode func() error
	if mode == "streamable-http" {
		tool := &agentsdk.FunctionTool{ToolName: "echo", ToolDescription: "interop echo", Schema: json.RawMessage(`{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}`), ReadOnly: true, Fn: func(context.Context, json.RawMessage) (string, error) { panic("policy bypass") }}
		server, err := wrapper.NewServerMode(nil, []agentsdk.Tool{tool}, wrapper.ServerToolPolicyFunc(func(_ context.Context, r wrapper.ServerToolRequest) (agentsdk.ToolResult, error) {
			if r.TenantID != tenant || len(r.RequestSHA256) != 64 {
				panic("policy context missing")
			}
			return agentsdk.ToolResult{Content: call(r.Arguments)}, nil
		}), wrapper.TenantResolverFunc(func(r *http.Request) (string, error) {
			if r.Header.Get("Authorization") != credential {
				return "", fmt.Errorf("unauthorized")
			}
			return tenant, nil
		}),
			wrapper.WithServerResources(wrapper.ServerResource{Definition: &protocol.Resource{URI: uri, Name: "resource"}, Read: func(_ context.Context, t string, _ *protocol.ReadResourceRequest) (*protocol.ReadResourceResult, error) {
				if t != tenant {
					panic("tenant")
				}
				return resource(), nil
			}}),
			wrapper.WithServerPrompts(wrapper.ServerPrompt{Definition: &protocol.Prompt{Name: "prompt"}, Get: func(_ context.Context, t string, _ *protocol.GetPromptRequest) (*protocol.GetPromptResult, error) {
				if t != tenant {
					panic("tenant")
				}
				return prompt(), nil
			}}))
		check(err)
		handler = server.Handler()
		closeMode = server.Close
	} else {
		// The pinned wrapper exports only Streamable HTTP ServerMode. These two modes
		// deliberately exercise its pinned protocol dependency, not a fictional wrapper API.
		server := protocol.NewServer(&protocol.Implementation{Name: "interop-go-protocol", Version: "1"}, nil)
		server.AddTool(&protocol.Tool{Name: "echo", Description: "interop echo", InputSchema: json.RawMessage(`{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}`), Annotations: &protocol.ToolAnnotations{ReadOnlyHint: true}}, func(_ context.Context, r *protocol.CallToolRequest) (*protocol.CallToolResult, error) {
			return &protocol.CallToolResult{Content: []protocol.Content{&protocol.TextContent{Text: call(r.Params.Arguments)}}}, nil
		})
		server.AddResource(&protocol.Resource{URI: uri, Name: "resource"}, func(context.Context, *protocol.ReadResourceRequest) (*protocol.ReadResourceResult, error) {
			return resource(), nil
		})
		server.AddPrompt(&protocol.Prompt{Name: "prompt"}, func(context.Context, *protocol.GetPromptRequest) (*protocol.GetPromptResult, error) {
			return prompt(), nil
		})
		if mode == "stdio" {
			check(server.Run(ctx, &protocol.StdioTransport{}))
			return
		}
		if mode != "sse" {
			panic("unknown mode")
		}
		sse := protocol.NewSSEHandler(func(*http.Request) *protocol.Server { return server }, nil)
		handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if r.Header.Get("Authorization") != credential {
				http.Error(w, "unauthorized", 401)
				return
			}
			sse.ServeHTTP(w, r)
		})
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	check(err)
	var deletes atomic.Int32
	httpServer := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		handler.ServeHTTP(w, r)
		if r.Method == http.MethodDelete && r.Header.Get("Authorization") == credential {
			deletes.Add(1)
		}
	}), ReadHeaderTimeout: 5 * time.Second}
	done := make(chan error, 1)
	go func() { done <- httpServer.Serve(listener) }()
	emit(map[string]any{"endpoint": "http://" + listener.Addr().String() + "/mcp"})
	stop := make(chan struct{})
	go func() { var b [1]byte; _, _ = os.Stdin.Read(b[:]); close(stop) }()
	select {
	case <-stop:
	case <-ctx.Done():
	}
	if closeMode != nil {
		check(closeMode())
	}
	// Close forcibly ends any surviving SSE stream; bounded even on a failed client.
	check(httpServer.Close())
	err = <-done
	if err != http.ErrServerClosed {
		check(err)
	}
	emit(map[string]any{"serverClosed": true, "deleteRequests": deletes.Load()})
}
func client(ctx context.Context) {
	if len(os.Args) != 4 {
		panic("client endpoint operation")
	}
	endpoint, op := os.Args[2], os.Args[3]
	u, err := url.Parse(endpoint)
	check(err)
	if u.Scheme != "http" || u.Hostname() != "127.0.0.1" {
		panic("loopback only")
	}
	work, err := os.MkdirTemp("", "interop-manager-")
	check(err)
	defer os.RemoveAll(work)
	manager, err := wrapper.NewManagerFromConfig(ctx, work, wrapper.Config{MCPServers: map[string]wrapper.ServerConfig{"rust": {Type: "streamable-http", URL: endpoint, TrustReadOnlyHint: true, AllowedTools: []string{"echo"}}}},
		wrapper.WithRemoteServers("rust"), wrapper.WithPrivateNetworkRemoteServers("rust"), wrapper.WithRemoteReadOnlyTools("rust", "echo"), wrapper.WithRemoteTenant(tenant),
		wrapper.WithRemoteHeaderProvider(wrapper.HeaderProviderFunc(func(_ context.Context, t, s string, _ *url.URL) (http.Header, error) {
			if t != tenant || s != "rust" {
				panic("credential scope")
			}
			return http.Header{"Authorization": []string{credential}}, nil
		})))
	check(err)
	defer manager.Close()
	var observed any
	switch op {
	case "tools":
		observed = manager.ToolDescriptors()
	case "resources":
		observed, err = manager.ListResources(ctx, "rust")
	case "prompts":
		observed, err = manager.ListPrompts(ctx, "rust")
	case "call":
		observed, err = manager.CallTool(ctx, wrapper.BuildToolName("rust", "echo"), map[string]any{"value": "cross-wire"})
	case "read":
		observed, err = manager.ReadResource(ctx, "rust", uri)
	case "prompt":
		observed, err = manager.GetPrompt(ctx, "rust", "prompt", map[string]string{})
	case "shutdown":
		observed = manager.ConnectedServerNames()
	default:
		panic("unknown operation")
	}
	check(err)
	check(manager.Close())
	emit(map[string]any{"observed": observed, "clientClosed": true})
}
