package mcp

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/gratefulagents/sdk/pkg/agentsdk"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	mcpsdk "github.com/modelcontextprotocol/go-sdk/mcp"
)

type referenceCase struct {
	Name     string          `json:"name"`
	Input    json.RawMessage `json:"input"`
	Kind     string          `json:"kind"`
	Generate string          `json:"generate"`
	Setup    string          `json:"setup"`
	Config   json.RawMessage `json:"config"`
	Layer    string          `json:"layer"`
}

func referenceHash(data []byte) string {
	hash := sha256.Sum256(data)
	return hex.EncodeToString(hash[:])
}

func TestReferenceCorpus(t *testing.T) {
	input, err := os.ReadFile(os.Getenv("MCP_REFERENCE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var cases struct {
		Schemas []referenceCase `json:"schemas"`
		Results []referenceCase `json:"results"`
		Configs []referenceCase `json:"configs"`
		Wire    []referenceCase `json:"wire"`
	}
	if err := json.Unmarshal(input, &cases); err != nil {
		t.Fatal(err)
	}
	out := map[string]any{"schemas": []any{}, "results": []any{}, "configs": []any{}}
	for _, c := range cases.Schemas {
		var schema any
		if err := json.Unmarshal(c.Input, &schema); err != nil {
			t.Fatal(err)
		}
		out["schemas"] = append(out["schemas"].([]any), map[string]any{"name": c.Name, "normalized": normalizeInputSchema(schema)})
	}
	for _, c := range cases.Results {
		workspace, outside := t.TempDir(), t.TempDir()
		if c.Setup == "symlink" {
			if err := os.Symlink(outside, filepath.Join(workspace, ".mcp")); err != nil {
				t.Fatal(err)
			}
		}
		wire := c.Input
		if c.Generate != "" {
			var block map[string]any
			if c.Generate == "long-text" {
				block = map[string]any{"type": "text", "text": strings.Repeat("界", 100000)}
			} else {
				block = map[string]any{"type": "image", "mimeType": "application/octet-stream", "data": base64.StdEncoding.EncodeToString(make([]byte, 10*1024*1024+1))}
			}
			if c.Generate == "oversized-malformed-blob" {
				block["data"] = block["data"].(string) + "!"
			}
			value := map[string]any{"content": []any{block}}
			if c.Kind == "resource" {
				delete(block, "type")
				block["uri"] = "test://item"
				if data, ok := block["data"]; ok {
					block["blob"] = data
					delete(block, "data")
				}
				value = map[string]any{"contents": []any{block}}
			}
			if c.Generate == "oversized-malformed-blob" {
				if c.Kind == "resource" {
					value["contents"] = []any{map[string]any{"blob": "aGk="}, block}
				} else {
					value["content"] = []any{map[string]any{"type": "image", "data": "aGk="}, block}
				}
			}
			wire, err = json.Marshal(value)
			if err != nil {
				t.Fatal(err)
			}
		}
		observation := map[string]any{"name": c.Name, "blobs": []any{}, "diagnostics": []any{}, "isError": false}
		var rendered string
		if c.Kind == "resource" {
			var result *mcpsdk.ReadResourceResult
			err = json.Unmarshal(wire, &result)
			if err == nil {
				rendered, err = FormatReadResourceResult(workspace, "server", result)
			} else {
				observation["decodeError"] = err.Error()
			}
		} else {
			var result *mcpsdk.CallToolResult
			err = json.Unmarshal(wire, &result)
			if err == nil {
				if result != nil {
					observation["isError"] = result.IsError
				}
				rendered, err = FormatCallToolResult(workspace, ToolDescriptor{ServerName: "server", ToolName: "tool"}, result)
			} else {
				observation["decodeError"] = err.Error()
			}
		}
		if err != nil && observation["decodeError"] == nil {
			t.Fatalf("%s: %v", c.Name, err)
		}
		if observation["decodeError"] == nil {
			var value any
			if json.Unmarshal([]byte(rendered), &value) != nil {
				value = rendered
			}
			var normalize func(any) any
			normalize = func(value any) any {
				switch v := value.(type) {
				case map[string]any:
					if path, ok := v["blobSavedTo"].(string); ok {
						data, err := os.ReadFile(path)
						if err != nil {
							t.Fatal(err)
						}
						stat, err := os.Stat(path)
						if err != nil {
							t.Fatal(err)
						}
						rel, err := filepath.Rel(workspace, path)
						if err != nil {
							t.Fatal(err)
						}
						observation["blobs"] = append(observation["blobs"].([]any), map[string]any{"bytes": len(data), "sha256": referenceHash(data), "mode": fmt.Sprintf("%03o", stat.Mode().Perm()), "confined": strings.HasPrefix(rel, ".mcp/blobs/")})
						for key, val := range v {
							if text, ok := val.(string); ok {
								v[key] = strings.ReplaceAll(text, path, "<blob>")
							}
						}
					}
					for _, key := range []string{"error", "text"} {
						if text, ok := v[key].(string); ok && (key == "error" || strings.HasPrefix(text, "Binary content could not be saved")) {
							text = strings.ReplaceAll(strings.ReplaceAll(text, workspace, "<workspace>"), outside, "<outside>")
							observation["diagnostics"] = append(observation["diagnostics"].([]any), text)
							v[key] = "<blob-error>"
						}
					}
					for key, val := range v {
						v[key] = normalize(val)
					}
					return v
				case []any:
					for i, val := range v {
						v[i] = normalize(val)
					}
					return v
				case string:
					if len(v) > 1024 {
						return map[string]any{"textBytes": len(v), "textSHA256": referenceHash([]byte(v))}
					}
				}
				return value
			}
			observation["rendered"] = normalize(value)
		}
		if observation["decodeError"] != nil {
			entries, err := os.ReadDir(workspace)
			if err != nil {
				t.Fatal(err)
			}
			observation["workspaceEmpty"] = len(entries) == 0
			if len(entries) != 0 {
				t.Fatalf("%s: decode failure wrote files", c.Name)
			}
		}
		if c.Setup == "symlink" {
			entries, err := os.ReadDir(outside)
			if err != nil {
				t.Fatal(err)
			}
			observation["outsideEmpty"] = len(entries) == 0
		}
		out["results"] = append(out["results"].([]any), observation)
	}
	for _, c := range cases.Configs {
		workspace := t.TempDir()
		path := filepath.Join(workspace, ".mcp.json")
		data := []byte(c.Config)
		if c.Setup == "oversized" {
			data = append(data, []byte(strings.Repeat(" ", 1024*1024))...)
		}
		if err := os.WriteFile(path, data, 0600); err != nil {
			t.Fatal(err)
		}
		if c.Setup == "symlink" {
			if err := os.Rename(path, path+".real"); err != nil {
				t.Fatal(err)
			}
			if err := os.Symlink(path+".real", path); err != nil {
				t.Fatal(err)
			}
		}
		cfg, exists, err := LoadConfig(path)
		observation := map[string]any{"name": c.Name, "loadAccept": err == nil, "exists": exists}
		if err != nil {
			observation["loadError"] = strings.ReplaceAll(err.Error(), workspace, "<workspace>")
		}
		if err == nil {
			server := cfg.MCPServers["bad"]
			opts := resolveManagerOptions(workspace, nil)
			switch c.Layer {
			case "remote":
				_, err := connectRemoteServer(context.Background(), "bad", server, opts)
				if err == nil {
					t.Fatal("unexpected host authorization")
				}
				observation["hostDenied"] = err.Error()
				for _, allowPrivate := range []bool{false, true} {
					_, err := validateRemoteEndpoint(context.Background(), server.URL, allowPrivate)
					key := "endpointPublic"
					if allowPrivate {
						key = "endpointPrivateOptIn"
					}
					entry := map[string]any{"accept": err == nil}
					if err != nil {
						entry["error"] = err.Error()
					}
					observation[key] = entry
				}
			case "dispatch":
				_, err := connectConfiguredServer(context.Background(), workspace, "bad", server, opts)
				if err == nil {
					t.Fatal("unexpected dispatch success")
				}
				observation["dispatchError"] = err.Error()
			case "env":
				filtered, _ := FilterCredentialEnv(server.Env, server.AllowEnv)
				observation["filteredEnv"] = filtered
			}
		}
		out["configs"] = append(out["configs"].([]any), observation)
	}
	out["wire"] = referenceWire(t, cases.Wire)
	data, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("MCP_REFERENCE_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
	t.Logf("observed %d schemas, %d results, %d configs, %d server requests against actual SDK", len(cases.Schemas), len(cases.Results), len(cases.Configs), len(cases.Wire))
}

func referenceWire(t *testing.T, cases []referenceCase) []any {
	var policyMu sync.Mutex
	var policyArguments json.RawMessage
	tool := &agentsdk.FunctionTool{ToolName: "lookup", Schema: json.RawMessage(`{"type":"object","properties":{"id":{"type":"string"}}}`)}
	mode, err := NewServerMode(&mcpsdk.Implementation{Name: "reference", Version: "1"}, []agentsdk.Tool{tool},
		ServerToolPolicyFunc(func(_ context.Context, request ServerToolRequest) (agentsdk.ToolResult, error) {
			policyMu.Lock()
			policyArguments = append(json.RawMessage(nil), request.Arguments...)
			policyMu.Unlock()
			var args map[string]any
			if err := json.Unmarshal(request.Arguments, &args); err != nil {
				t.Fatal(err)
			}
			if args["deny"] == true {
				return agentsdk.ToolResult{}, fmt.Errorf("TOP_SECRET policy details")
			}
			text := "policy result"
			if args["empty"] == true {
				text = ""
			}
			return agentsdk.ToolResult{Content: text, IsError: args["error"] == true}, nil
		}), TenantResolverFunc(func(*http.Request) (string, error) { return "tenant", nil }),
		WithServerResources(ServerResource{Definition: &mcpsdk.Resource{URI: "memory://note/1", Name: ""}, Read: func(context.Context, string, *mcpsdk.ReadResourceRequest) (*mcpsdk.ReadResourceResult, error) {
			return &mcpsdk.ReadResourceResult{}, nil
		}}),
		WithServerPrompts(ServerPrompt{Definition: &mcpsdk.Prompt{Name: "empty"}, Get: func(context.Context, string, *mcpsdk.GetPromptRequest) (*mcpsdk.GetPromptResult, error) {
			return &mcpsdk.GetPromptResult{}, nil
		}},
			ServerPrompt{Definition: &mcpsdk.Prompt{Name: "optional", Arguments: []*mcpsdk.PromptArgument{{Name: "subject"}}}, Get: func(context.Context, string, *mcpsdk.GetPromptRequest) (*mcpsdk.GetPromptResult, error) {
				return &mcpsdk.GetPromptResult{}, nil
			}}))
	if err != nil {
		t.Fatal(err)
	}
	defer mode.Close()
	server := httptest.NewServer(mode.Handler())
	defer server.Close()
	client := mcpsdk.NewClient(&mcpsdk.Implementation{Name: "reference", Version: "1"}, nil)
	session, err := client.Connect(t.Context(), &mcpsdk.StreamableClientTransport{Endpoint: server.URL, DisableStandaloneSSE: true}, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close()
	observations := []any{}
	for _, c := range cases {
		policyMu.Lock()
		policyArguments = nil
		policyMu.Unlock()
		var message map[string]any
		if err := json.Unmarshal(c.Input, &message); err != nil {
			t.Fatal(err)
		}
		message["jsonrpc"] = "2.0"
		message["id"] = 7
		data, err := json.Marshal(message)
		if err != nil {
			t.Fatal(err)
		}
		request, err := http.NewRequestWithContext(t.Context(), http.MethodPost, server.URL, strings.NewReader(string(data)))
		if err != nil {
			t.Fatal(err)
		}
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Accept", "application/json, text/event-stream")
		request.Header.Set("Mcp-Session-Id", session.ID())
		request.Header.Set("Mcp-Protocol-Version", "2025-06-18")
		response, err := http.DefaultClient.Do(request)
		if err != nil {
			t.Fatal(err)
		}
		body, err := io.ReadAll(response.Body)
		response.Body.Close()
		if err != nil {
			t.Fatal(err)
		}
		var value any
		if strings.HasPrefix(response.Header.Get("Content-Type"), "text/event-stream") {
			for _, line := range strings.Split(string(body), "\n") {
				if strings.HasPrefix(line, "data: ") {
					if err := json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &value); err != nil {
						t.Fatal(err)
					}
				}
			}
		} else if err := json.Unmarshal(body, &value); err != nil {
			t.Fatalf("%s: %s", c.Name, body)
		}
		if value == nil {
			t.Fatalf("no response for %s: %s", c.Name, body)
		}
		observation := map[string]any{"name": c.Name, "status": response.StatusCode, "response": value}
		policyMu.Lock()
		if policyArguments != nil {
			observation["policyArguments"] = policyArguments
		}
		policyMu.Unlock()
		observations = append(observations, observation)
	}
	return observations
}
