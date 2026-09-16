// SPDX-License-Identifier: GPL-3.0-only
package main

import (
	"bytes"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestASTEdges(t *testing.T) {
	const src = "package sample\nimport \"encoding/json\"\n" + `
 type Model[T any] interface { Call(value T) (string, error) }
 type Base struct { A, B int ` + "`json:\"field\"`" + `; private bool }
 type Alias = Base
 type Wrapper struct { Base }
 func (*Base) Name() string { return "fixture" }
 func (*Base) InputSchema() json.RawMessage { return json.RawMessage(` + "`{\"type\":\"object\",\"properties\":{\"n\":{\"type\":\"integer\",\"default\":3}}}`" + `) }
 func NewBase() *Base { return nil }
 var Make = NewBase
 const ( One = iota; Two )
 `
	g := &Generator{fs: token.NewFileSet(), routes: map[string]Route{"sample": {"sdk::sample", "test-owner"}}, types: map[string]Row{}, members: map[string][]string{}, methods: map[string]map[string][]Row{}, inv: Inventory{Records: map[string][]Row{}}}
	a, err := parser.ParseFile(g.fs, "sample.go", src, parser.ParseComments)
	if err != nil {
		t.Fatal(err)
	}
	f := &File{path: "sample.go", dir: "sample", ast: a, src: []byte(src), imports: map[string]string{"json": "encoding/json"}}
	g.parse(f)
	g.tools()
	byName := map[string]Row{}
	for _, r := range g.inv.Records["apis"] {
		byName[r["name"].(string)] = r
	}
	for _, name := range []string{"Base.A", "Base.B", "Base.private", "Model.Call", "Wrapper.Base"} {
		if byName[name] == nil {
			t.Errorf("missing member %s", name)
		}
	}
	if byName["Base.A"]["tag"] != `json:"field"` {
		t.Fatal("field tag lost")
	}
	if byName["Alias"]["resolved_type_id"] != byName["Base"]["id"] {
		t.Fatal("alias resolution lost")
	}
	if len(byName["Alias"]["resolved_member_ids"].([]string)) != 5 {
		t.Fatal("alias members incomplete")
	}
	if !strings.Contains(byName["Make"]["target_signature"].(string), "func NewBase() *Base") {
		t.Fatal("function-valued API signature not linked")
	}
	if byName["Two"]["const_group_index"] != 1 || !strings.Contains(byName["Two"]["effective_spec"].(string), "iota") {
		t.Fatal("implicit const lost")
	}
	if len(g.inv.Records["tools"]) != 2 {
		t.Fatal("expected direct and promoted tool, not alias duplicate")
	}
	for _, r := range g.inv.Records["tools"] {
		if r["schema_status"] != "literal-json-candidates-with-source" {
			t.Fatal("schema not decoded")
		}
	}
}

func TestPinnedBaseline(t *testing.T) {
	root := "repos/sdk"
	routes := "scripts/inventory/routes.json"
	if _, err := os.Stat(root); err != nil {
		root = "../../repos/sdk"
		routes = "routes.json"
	}
	g := generate(root, routes)
	if len(g.sources) != 460 || len(g.files) != 398 {
		t.Fatalf("unexpected tracked breadth: %d sources, %d Go files", len(g.sources), len(g.files))
	}
	want := map[string]int{"apis": 5859, "packages": 74, "tools": 62, "cli_flags": 49, "tests": 1182, "os_constraints": 31}
	for kind, n := range want {
		if len(g.inv.Records[kind]) != n {
			t.Errorf("%s: want %d got %d", kind, n, len(g.inv.Records[kind]))
		}
	}
	ids := map[string]bool{}
	acceptance := map[string]bool{}
	for kind, rows := range g.inv.Records {
		for _, r := range rows {
			id := r["id"].(string)
			if ids[id] {
				t.Errorf("duplicate id %s", id)
			}
			ids[id] = true
			aid := r["acceptance_id"].(string)
			if acceptance[aid] {
				t.Errorf("duplicate acceptance %s", aid)
			}
			acceptance[aid] = true
			for _, key := range []string{"rust_module", "role_owner", "implementation_status", "verification_status", "acceptance_contract"} {
				if r[key] == nil || r[key] == "" {
					t.Errorf("%s missing %s", id, key)
				}
			}
			if r["implementation_status"] != "not_implemented" || r["verification_status"] != "not_run" {
				t.Errorf("false parity claim in %s", id)
			}
			if kind == "apis" && r["alias"] == true && r["resolved_type_id"] == nil {
				t.Errorf("unresolved pinned alias %s", id)
			}
		}
	}
	for _, r := range g.inv.Records["apis"] {
		for _, key := range []string{"parent", "target_id", "resolved_type_id"} {
			if ref, ok := r[key].(string); ok && !ids[ref] {
				t.Errorf("dangling %s: %s", key, ref)
			}
		}
		for _, key := range []string{"member_ids", "resolved_member_ids"} {
			if refs, ok := r[key].([]string); ok {
				for _, ref := range refs {
					if !ids[ref] {
						t.Errorf("dangling member %s", ref)
					}
				}
			}
		}
	}
	for _, s := range g.sources {
		b, err := os.ReadFile(filepath.Join(root, s.Path))
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(b, []byte(s.Content)) || hash(s.Content) != s.SHA256 {
			t.Errorf("source fidelity lost %s", s.Path)
		}
	}
	for _, r := range g.inv.Records["tools"] {
		methods := r["methods"].(map[string][]Row)
		for _, key := range []string{"Name", "Description", "InputSchema", "Execute"} {
			if len(methods[key]) == 0 {
				t.Errorf("tool %v missing %s", r["type"], key)
			}
		}
	}
}

func TestUnmappedFails(t *testing.T) {
	defer func() {
		if recover() == nil {
			t.Fatal("unmapped capability silently accepted")
		}
	}()
	g := &Generator{routes: map[string]Route{}}
	g.add("apis", "new", "unknown", Row{})
}
