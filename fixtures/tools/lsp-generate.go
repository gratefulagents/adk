package lsp

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Loaded as an additional package test through go -overlay; SDK files stay untouched.
func TestGenerateDifferential(t *testing.T) {
	fixture := os.Getenv("LSP_FIXTURES")
	data, err := os.ReadFile(filepath.Join(fixture, "lsp-cases.json"))
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		SDKCommit  string   `json:"sdk_commit"`
		Operations []string `json:"operations"`
		Cases      []struct {
			Name      string          `json:"name"`
			Operation string          `json:"operation"`
			Raw       json.RawMessage `json:"raw"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatal(err)
	}
	workspace := t.TempDir()
	var rows []map[string]any
	for _, c := range corpus.Cases {
		op, err := normalizeOperation(c.Operation)
		if err != nil {
			t.Fatal(err)
		}
		raw := json.RawMessage(strings.ReplaceAll(string(c.Raw), "@ROOT@", workspace))
		result, err := parseResult(op, raw, workspace)
		row := map[string]any{"name": c.Name, "is_error": err != nil}
		if err != nil {
			row["error"] = err.Error()
		} else {
			for i := range result.Diagnostics {
				result.Diagnostics[i].FilePath = filepath.Join(workspace, "sample.txt")
			}
			encoded, err := json.Marshal(result)
			if err != nil {
				t.Fatal(err)
			}
			var stable any
			if err := json.Unmarshal([]byte(strings.ReplaceAll(string(encoded), workspace, "@ROOT@")), &stable); err != nil {
				t.Fatal(err)
			}
			row["result"] = stable
		}
		rows = append(rows, row)
	}
	var operations []map[string]any
	for _, op := range corpus.Operations {
		normalized, err := normalizeOperation(op)
		row := map[string]any{"input": op, "is_error": err != nil}
		if err != nil {
			row["error"] = err.Error()
		} else {
			row["normalized"] = normalized
		}
		operations = append(operations, row)
	}
	tool := NewTool(Config{})
	output := map[string]any{"sdk_commit": corpus.SDKCommit, "cases": rows, "operations": operations, "definition": map[string]any{"name": tool.Name(), "description": tool.Description(), "input_schema": tool.InputSchema(), "read_only": tool.IsReadOnly(), "requires_approval": tool.NeedsApproval()}}
	encoded, err := json.MarshalIndent(output, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(fixture, "lsp-expected.json"), append(encoded, '\n'), 0644); err != nil {
		t.Fatal(err)
	}
	t.Logf("generated %d result cases, %d operation cases, one complete tool definition", len(rows), len(operations))
}
