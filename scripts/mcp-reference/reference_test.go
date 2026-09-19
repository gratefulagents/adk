package mcp

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
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
	data, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("MCP_REFERENCE_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
	t.Logf("observed %d schemas, %d results, %d configs against actual SDK", len(cases.Schemas), len(cases.Results), len(cases.Configs))
}
