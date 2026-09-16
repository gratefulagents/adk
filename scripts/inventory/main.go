// SPDX-License-Identifier: GPL-3.0-only
// Source excerpts in generated artifacts retain the upstream SDK license.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"path"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

const pin = "1dc92b73900fac74dc357a938e4b5eee6392b418"
const module = "github.com/gratefulagents/sdk"
const version = "sdk-v0.0.115"

type Row map[string]any
type Route struct {
	Module string `json:"rust_module"`
	Owner  string `json:"role_owner"`
}
type Source struct {
	Path    string `json:"path"`
	SHA256  string `json:"sha256"`
	Content string `json:"content"`
}
type File struct {
	path, dir string
	ast       *ast.File
	src       []byte
	imports   map[string]string
	test      bool
}
type Inventory struct {
	SchemaVersion int              `json:"schema_version"`
	SDKVersion    string           `json:"sdk_version"`
	Commit        string           `json:"commit"`
	Upstream      string           `json:"upstream"`
	License       string           `json:"source_license"`
	Counts        map[string]int   `json:"counts"`
	Records       map[string][]Row `json:"records"`
}
type Generator struct {
	fs      *token.FileSet
	inv     Inventory
	files   []*File
	sources []Source
	routes  map[string]Route
	types   map[string]Row
	members map[string][]string
	methods map[string]map[string][]Row
}

func fail(err error) {
	if err != nil {
		panic(err)
	}
}
func hash(s string) string { h := sha256.Sum256([]byte(s)); return hex.EncodeToString(h[:]) }
func git(root string, args ...string) string {
	out, err := exec.Command("git", append([]string{"-C", root}, args...)...).CombinedOutput()
	if err != nil {
		panic(string(out))
	}
	return strings.TrimSpace(string(out))
}
func render(fs *token.FileSet, n ast.Node) string {
	switch x := n.(type) {
	case *ast.Field:
		names := []string{}
		for _, name := range x.Names {
			names = append(names, name.Name)
		}
		prefix := ""
		if len(names) > 0 {
			prefix = strings.Join(names, ", ") + " "
		}
		result := prefix + render(fs, x.Type)
		if x.Tag != nil {
			result += " " + x.Tag.Value
		}
		return result
	case *ast.FieldList:
		fields := []string{}
		for _, field := range x.List {
			fields = append(fields, render(fs, field))
		}
		return "(" + strings.Join(fields, ", ") + ")"
	case *ast.Comment:
		return x.Text
	}
	var b bytes.Buffer
	fail(format.Node(&b, fs, n))
	return b.String()
}
func literal(e ast.Expr) string {
	if x, ok := e.(*ast.BasicLit); ok && x.Kind == token.STRING {
		s, err := strconv.Unquote(x.Value)
		fail(err)
		return s
	}
	return ""
}
func receiver(e ast.Expr) string {
	switch x := e.(type) {
	case *ast.Ident:
		return x.Name
	case *ast.StarExpr:
		return receiver(x.X)
	case *ast.IndexExpr:
		return receiver(x.X)
	case *ast.IndexListExpr:
		return receiver(x.X)
	}
	return ""
}
func qualified(f *File, e ast.Expr) string {
	switch x := e.(type) {
	case *ast.Ident:
		return f.dir + "." + x.Name
	case *ast.StarExpr:
		return qualified(f, x.X)
	case *ast.SelectorExpr:
		if p, ok := x.X.(*ast.Ident); ok {
			imp := f.imports[p.Name]
			if strings.HasPrefix(imp, module+"/") {
				return strings.TrimPrefix(imp, module+"/") + "." + x.Sel.Name
			}
			return imp + "." + x.Sel.Name
		}
	}
	return ""
}
func (g *Generator) add(kind, key, dir string, r Row) Row {
	route, ok := g.routes[dir]
	if source, exists := r["source"].(string); exists {
		if specific, exists := g.routes[source]; exists {
			route = specific
		}
	}
	if !ok {
		panic("unmapped package/path: " + dir)
	}
	if descriptor, exists := r["descriptor"].(Row); exists {
		name := strings.Trim(descriptor["Family"].(string), `"`)
		selected, exists := g.routes["capability:"+name]
		if !exists {
			panic("unmapped capability: " + name)
		}
		route = selected
	}
	r["id"] = kind + ":" + key
	r["acceptance_id"] = "SDK-" + strings.ToUpper(hash(kind + ":" + key)[:16])
	r["rust_module"] = route.Module
	r["role_owner"] = route.Owner
	r["implementation_status"] = "not_implemented"
	r["verification_status"] = "not_run"
	r["acceptance_contract"] = "source-parity-v1"
	g.inv.Records[kind] = append(g.inv.Records[kind], r)
	return r
}
func (g *Generator) evidence(f *File, n ast.Node) Row {
	return Row{"source": f.path, "line": g.fs.Position(n.Pos()).Line, "end_line": g.fs.Position(n.End()).Line, "start_byte": g.fs.Position(n.Pos()).Offset, "end_byte": g.fs.Position(n.End()).Offset}
}
func (g *Generator) expression(f *File, n ast.Node) Row {
	r := g.evidence(f, n)
	r["expression"] = render(g.fs, n)
	return r
}
func (g *Generator) api(f *File, n ast.Node, name, kind, signature string) Row {
	r := g.evidence(f, n)
	r["package"] = f.dir
	r["name"] = name
	r["kind"] = kind
	r["signature"] = signature
	r["visibility"] = "internal"
	if strings.HasPrefix(f.dir, "pkg/") && !strings.Contains(f.dir, "/internal/") {
		r["visibility"] = "public-package"
	}
	return g.add("apis", f.dir+"."+name+"@"+f.path, f.dir, r)
}
func (g *Generator) parse(f *File) {
	for _, d := range f.ast.Decls {
		switch d := d.(type) {
		case *ast.GenDecl:
			var prior *ast.ValueSpec
			for index, s := range d.Specs {
				switch s := s.(type) {
				case *ast.TypeSpec:
					if f.test {
						continue
					}
					r := g.api(f, s, s.Name.Name, "type", render(g.fs, s))
					r["exported"] = s.Name.IsExported()
					r["doc"] = d.Doc.Text() + s.Doc.Text()
					r["alias"] = s.Assign.IsValid()
					r["target"] = qualified(f, s.Type)
					g.types[f.dir+"."+s.Name.Name] = r
					var fields *ast.FieldList
					var memberKind string
					switch t := s.Type.(type) {
					case *ast.StructType:
						fields = t.Fields
						memberKind = "field"
					case *ast.InterfaceType:
						fields = t.Methods
						memberKind = "interface-method"
					}
					if fields != nil {
						for _, field := range fields.List {
							names := []string{}
							for _, n := range field.Names {
								names = append(names, n.Name)
							}
							if len(names) == 0 {
								names = []string{render(g.fs, field.Type)}
							}
							for _, name := range names {
								mr := g.api(f, field, s.Name.Name+"."+name, memberKind, render(g.fs, field))
								mr["parent"] = r["id"]
								if strings.HasSuffix(s.Name.Name, "Features") {
									cr := g.evidence(f, field)
									cr["name"] = s.Name.Name + "." + name
									cr["api_id"] = mr["id"]
									g.add("capabilities", f.dir+"."+s.Name.Name+"."+name, f.dir, cr)
								}
								mr["exported"] = ast.IsExported(name)
								mr["embedded"] = len(field.Names) == 0
								mr["type"] = render(g.fs, field.Type)
								mr["doc"] = field.Doc.Text() + field.Comment.Text()
								if field.Tag != nil {
									mr["tag"] = literal(field.Tag)
								}
								if len(field.Names) == 0 {
									mr["target"] = qualified(f, field.Type)
								}
								g.members[f.dir+"."+s.Name.Name] = append(g.members[f.dir+"."+s.Name.Name], mr["id"].(string))
							}
						}
					}
				case *ast.ValueSpec:
					if f.test {
						continue
					}
					effective := s
					if len(s.Values) == 0 && d.Tok == token.CONST && prior != nil {
						effective = prior
					}
					prior = effective
					for valueIndex, n := range s.Names {
						if !n.IsExported() {
							continue
						}
						r := g.api(f, s, n.Name, d.Tok.String(), render(g.fs, s))
						r["exported"] = true
						r["doc"] = d.Doc.Text() + s.Doc.Text()
						r["effective_spec"] = render(g.fs, effective)
						if valueIndex < len(effective.Values) {
							r["target"] = qualified(f, effective.Values[valueIndex])
						}
						r["const_group_index"] = index
					}
				}
			}
		case *ast.FuncDecl:
			name := d.Name.Name
			rec := ""
			if d.Recv != nil {
				rec = receiver(d.Recv.List[0].Type)
				name = rec + "." + name
			}
			if f.test {
				if d.Recv == nil && (strings.HasPrefix(name, "Test") || strings.HasPrefix(name, "Benchmark") || strings.HasPrefix(name, "Fuzz") || strings.HasPrefix(name, "Example")) {
					r := g.expression(f, d)
					delete(r, "expression")
					r["name"] = name
					r["package"] = f.dir
					r["signature"] = render(g.fs, d.Type)
					g.add("tests", f.path+":"+name, f.dir, r)
				}
				continue
			}
			var r Row
			if d.Name.IsExported() {
				copy := *d
				copy.Body = nil
				r = g.api(f, d, name, "function", render(g.fs, &copy))
				r["exported"] = true
				r["doc"] = d.Doc.Text()
				if rec != "" {
					r["kind"] = "method"
					r["receiver"] = render(g.fs, d.Recv)
					g.members[f.dir+"."+rec] = append(g.members[f.dir+"."+rec], r["id"].(string))
				}
			}
			if rec != "" {
				if g.methods[f.dir+"."+rec] == nil {
					g.methods[f.dir+"."+rec] = map[string][]Row{}
				}
				mr := g.expression(f, d)
				mr["returns"] = []string{}
				returns := []string{}
				literals := []string{}
				if d.Body != nil {
					ast.Inspect(d.Body, func(n ast.Node) bool {
						if ret, ok := n.(*ast.ReturnStmt); ok {
							for _, v := range ret.Results {
								returns = append(returns, render(g.fs, v))
								if s := literal(v); s != "" {
									literals = append(literals, s)
								}
							}
						}
						return true
					})
				}
				mr["returns"] = returns
				mr["literal_returns"] = literals
				g.methods[f.dir+"."+rec][d.Name.Name] = append(g.methods[f.dir+"."+rec][d.Name.Name], mr)
			}
			lower := strings.ToLower(name)
			if strings.Contains(lower, "default") || strings.Contains(lower, "config") || strings.Contains(lower, "registr") || strings.Contains(lower, "build") || strings.Contains(lower, "feature") {
				cr := g.evidence(f, d)
				cr["name"] = name
				cr["category"] = "configuration-constructor-or-registration"
				g.add("configuration", f.path+":"+name, f.dir, cr)
			}
		}
	}
	g.scan(f)
}
func (g *Generator) scan(f *File) {
	initializers := map[string]string{}
	ast.Inspect(f.ast, func(n ast.Node) bool {
		if c, ok := n.(*ast.CompositeLit); ok && c.Type != nil && render(g.fs, c.Type) == "cliConfig" {
			for _, e := range c.Elts {
				if kv, ok := e.(*ast.KeyValueExpr); ok {
					initializers["cfg."+render(g.fs, kv.Key)] = render(g.fs, kv.Value)
				}
			}
		}
		return true
	})
	ast.Inspect(f.ast, func(n ast.Node) bool {
		if n == nil {
			return true
		}
		key := fmt.Sprintf("%s:%d", f.path, g.fs.Position(n.Pos()).Offset)
		if expr, ok := n.(*ast.BinaryExpr); ok && strings.Contains(render(g.fs, expr), "runtime.GOOS") {
			r := g.expression(f, expr)
			r["test_only"] = f.test
			g.add("os_runtime_checks", key, f.dir, r)
		}
		if c, ok := n.(*ast.CallExpr); ok {
			name := render(g.fs, c.Fun)
			leaf := name
			if s, ok := c.Fun.(*ast.SelectorExpr); ok {
				leaf = s.Sel.Name
			}
			if !f.test && strings.HasPrefix(f.dir, "cmd/") && strings.HasPrefix(name, "fs.") && strings.HasSuffix(leaf, "Var") && len(c.Args) >= 4 {
				r := g.expression(f, c)
				r["flag"] = literal(c.Args[1])
				r["go_type"] = strings.TrimSuffix(leaf, "Var")
				r["binding"] = render(g.fs, c.Args[0])
				def := render(g.fs, c.Args[2])
				r["default_expression"] = def
				if v, ok := initializers[def]; ok {
					r["initializer"] = v
				} else {
					r["initializer"] = "0"
					if leaf == "StringVar" {
						r["initializer"] = `""`
					}
					if leaf == "BoolVar" {
						r["initializer"] = "false"
					}
					r["initializer_source"] = "implicit Go zero value in cliConfig"
				}
				r["help"] = literal(c.Args[3])
				g.add("cli_flags", key, f.dir, r)
			}
			if name == "os.Getenv" || name == "os.LookupEnv" || name == "envOr" || name == "envInt" || name == "envBool" {
				r := g.expression(f, c)
				r["test_only"] = f.test
				if len(c.Args) > 0 {
					r["key_expression"] = render(g.fs, c.Args[0])
					r["key"] = literal(c.Args[0])
				}
				if len(c.Args) > 1 {
					r["fallback_expression"] = render(g.fs, c.Args[1])
				}
				g.add("environment", key, f.dir, r)
			}
			if !f.test && (leaf == "Register" || leaf == "RegisterTool" || leaf == "AddTool" || leaf == "NewFunctionTool" || leaf == "BuildSubAgentTaskTools" || strings.HasPrefix(leaf, "With") && strings.Contains(leaf, "Tool")) {
				r := g.expression(f, c)
				r["call"] = name
				g.add("registrations", key, f.dir, r)
			}
			if f.test && leaf == "Run" && len(c.Args) > 0 {
				r := g.evidence(f, c)
				r["name_expression"] = render(g.fs, c.Args[0])
				g.add("subtests", key, f.dir, r)
			}
		}
		if !f.test {
			if c, ok := n.(*ast.CompositeLit); ok {
				descriptor := Row{}
				for _, e := range c.Elts {
					if kv, ok := e.(*ast.KeyValueExpr); ok {
						k := render(g.fs, kv.Key)
						if k == "Family" || k == "Classification" || k == "Options" {
							descriptor[k] = render(g.fs, kv.Value)
						}
					}
				}
				if descriptor["Family"] != nil {
					r := g.expression(f, c)
					r["descriptor"] = descriptor
					g.add("capabilities", key, f.dir, r)
				}
			}
			if c, ok := n.(*ast.CompositeLit); ok && c.Type != nil {
				typ := render(g.fs, c.Type)
				if strings.HasSuffix(typ, "Tool") || strings.HasSuffix(typ, "RegistryCapability") {
					r := g.expression(f, c)
					r["type"] = typ
					g.add("registrations", key, f.dir, r)
				}
				if strings.Contains(typ, "Config") || strings.Contains(typ, "Settings") || strings.Contains(typ, "Features") || strings.Contains(typ, "FunctionTool") {
					r := g.expression(f, c)
					r["type"] = typ
					g.add("defaults", key, f.dir, r)
				}
			}
		}
		return true
	})
	for _, cg := range f.ast.Comments {
		for _, c := range cg.List {
			if strings.HasPrefix(c.Text, "//go:build") || strings.HasPrefix(c.Text, "// +build") {
				r := g.expression(f, c)
				g.add("os_constraints", fmt.Sprintf("%s:%d", f.path, g.fs.Position(c.Pos()).Offset), f.dir, r)
			}
		}
	}
}
func (g *Generator) tools() {
	byID := map[string]Row{}
	for _, r := range g.inv.Records["apis"] {
		byID[r["id"].(string)] = r
	}
	var inherited func(string, map[string]bool) map[string][]Row
	inherited = func(key string, seen map[string]bool) map[string][]Row {
		out := map[string][]Row{}
		if seen[key] {
			return out
		}
		seen[key] = true
		if typ := g.types[key]; typ != nil && typ["alias"] == true {
			return out
		}
		for _, id := range g.members[key] {
			member := byID[id]
			if member["embedded"] == true {
				target, _ := member["target"].(string)
				for name, rows := range inherited(target, seen) {
					out[name] = append(out[name], rows...)
				}
			}
		}
		for name, rows := range g.methods[key] {
			out[name] = rows
		}
		delete(seen, key)
		return out
	}
	expanded := map[string]map[string][]Row{}
	for key := range g.types {
		expanded[key] = inherited(key, map[string]bool{})
	}
	for key, methods := range g.methods {
		if _, ok := expanded[key]; !ok {
			expanded[key] = methods
		}
	}
	g.methods = expanded
	keys := []string{}
	for k, m := range g.methods {
		if len(m["InputSchema"]) > 0 {
			keys = append(keys, k)
		}
	}
	sort.Strings(keys)
	for _, key := range keys {
		methods := g.methods[key]
		dot := strings.LastIndex(key, ".")
		dir := key[:dot]
		r := Row{"source": methods["InputSchema"][0]["source"], "type": key, "methods": methods, "registration_status": "definition; availability conditional on registry/runtime/host configuration", "schema_status": "source-expression", "defaults_status": "schema plus Execute/constructor source; not evaluated"}
		names := []string{}
		for _, m := range methods["Name"] {
			names = append(names, m["literal_returns"].([]string)...)
		}
		r["literal_names"] = names
		schemas := []json.RawMessage{}
		for _, m := range methods["InputSchema"] {
			expr := m["expression"].(string)
			parsed, err := parser.ParseFile(token.NewFileSet(), "schema.go", "package p\n"+expr, 0)
			fail(err)
			ast.Inspect(parsed, func(n ast.Node) bool {
				if call, ok := n.(*ast.CallExpr); ok {
					if sel, ok := call.Fun.(*ast.SelectorExpr); ok && sel.Sel.Name == "RawMessage" && len(call.Args) == 1 {
						s := literal(call.Args[0])
						if s != "" {
							if !json.Valid([]byte(s)) {
								panic("invalid literal schema: " + key)
							}
							schemas = append(schemas, json.RawMessage(s))
						}
					}
				}
				return true
			})
		}
		r["literal_schemas"] = schemas
		if len(schemas) > 0 {
			r["schema_status"] = "literal-json-candidates-with-source"
		}
		tool := g.add("tools", key, dir, r)
		for i, raw := range schemas {
			var schema struct {
				Properties map[string]json.RawMessage `json:"properties"`
				Required   []string                   `json:"required"`
			}
			fail(json.Unmarshal(raw, &schema))
			for name, property := range schema.Properties {
				required := false
				for _, req := range schema.Required {
					if req == name {
						required = true
					}
				}
				pr := Row{"source": r["source"], "tool_id": tool["id"], "name": name, "schema_candidate_index": i, "schema": property, "required": required, "default_semantics": "only JSON default is structured; prose and runtime Execute source may supply additional defaults"}
				g.add("tool_parameters", fmt.Sprintf("%s:%d:%s", key, i, name), dir, pr)
			}
		}

	}
	symbols := map[string]Row{}
	for _, r := range g.inv.Records["apis"] {
		if r["kind"] != "field" && r["kind"] != "interface-method" {
			symbols[r["package"].(string)+"."+r["name"].(string)] = r
		}
	}
	for _, r := range g.inv.Records["apis"] {
		if target, ok := r["target"].(string); ok {
			if tr := symbols[target]; tr != nil {
				r["target_id"] = tr["id"]
				r["target_signature"] = tr["signature"]
				if r["alias"] == true || r["kind"] == "var" {
					r["rust_module"] = tr["rust_module"]
					r["role_owner"] = tr["role_owner"]
				}
			}
		}
	}
	for key, r := range g.types {
		r["member_ids"] = g.members[key]
		if alias, ok := r["alias"].(bool); ok && alias {
			target, _ := r["target"].(string)
			seen := map[string]bool{}
			for target != "" && !seen[target] {
				seen[target] = true
				if tr, ok := g.types[target]; ok {
					r["resolved_type_id"] = tr["id"]
					r["resolved_member_ids"] = g.members[target]
					if tr["alias"] == true {
						target, _ = tr["target"].(string)
						continue
					}
				}
				break
			}
		}
	}
}
func writeJSON(v any) []byte {
	b, err := json.MarshalIndent(v, "", "  ")
	fail(err)
	return append(b, '\n')
}
func generate(root, routesPath string) *Generator {
	if got := git(root, "rev-parse", "HEAD"); got != pin {
		panic("SDK pin mismatch: " + got)
	}
	if got := git(root, "status", "--porcelain", "--untracked-files=all"); got != "" {
		panic("SDK checkout must be clean: " + got)
	}
	if git(root, "rev-parse", "v0.0.115^{commit}") != pin {
		panic("SDK tag mismatch")
	}
	g := &Generator{fs: token.NewFileSet(), routes: map[string]Route{}, types: map[string]Row{}, members: map[string][]string{}, methods: map[string]map[string][]Row{}, inv: Inventory{SchemaVersion: 1, SDKVersion: "v0.0.115", Commit: pin, Upstream: "https://github.com/gratefulagents/sdk", License: "GPL-3.0 (upstream LICENSE; excerpts are derived from upstream)", Counts: map[string]int{}, Records: map[string][]Row{}}}
	b, err := os.ReadFile(routesPath)
	fail(err)
	fail(json.Unmarshal(b, &g.routes))
	files := strings.Split(git(root, "ls-files"), "\n")
	packages := map[string][]string{}
	for _, p := range files {
		b, err := os.ReadFile(filepath.Join(root, p))
		fail(err)
		g.sources = append(g.sources, Source{p, hash(string(b)), string(b)})
		dir := path.Dir(p)
		if _, ok := g.routes[dir]; !ok {
			panic("missing route: " + dir)
		}
		r := Row{"source": p, "sha256": hash(string(b)), "bytes": len(b)}
		g.add("files", p, dir, r)
		if strings.HasPrefix(p, "examples/") {
			g.add("examples", p, dir, Row{"source": p})
		}
		if strings.HasPrefix(p, "eval/") || strings.HasPrefix(p, "scripts/") || strings.HasPrefix(p, ".github/workflows/") || p == "Makefile" {
			g.add("evals_and_automation", p, dir, Row{"source": p})
		}
		if !strings.HasSuffix(p, ".go") {
			g.add("artifacts", p, dir, Row{"source": p})
			continue
		}
		a, err := parser.ParseFile(g.fs, p, b, parser.ParseComments|parser.AllErrors)
		fail(err)
		f := &File{p, dir, a, b, map[string]string{}, strings.HasSuffix(p, "_test.go")}
		for _, i := range a.Imports {
			imp := literal(i.Path)
			alias := path.Base(imp)
			if i.Name != nil {
				alias = i.Name.Name
			}
			f.imports[alias] = imp
		}
		g.files = append(g.files, f)
		packages[dir+"|"+a.Name.Name] = append(packages[dir+"|"+a.Name.Name], p)
	}
	for _, f := range g.files {
		g.parse(f)
	}
	g.tools()
	for key, files := range packages {
		parts := strings.Split(key, "|")
		g.add("packages", key, parts[0], Row{"directory": parts[0], "name": parts[1], "import_path": module + "/" + parts[0], "files": files})
	}
	groups := map[string]Row{}
	for _, rows := range g.inv.Records {
		for _, r := range rows {
			mod := r["rust_module"].(string)
			id := "verification_groups:" + mod
			r["verification_group_id"] = id
			if groups[mod] == nil {
				groups[mod] = Row{"id": id, "acceptance_id": "SDK-" + strings.ToUpper(hash(id)[:16]), "acceptance_contract": "source-parity-v1", "rust_module": mod, "role_owner": r["role_owner"], "implementation_status": "not_implemented", "verification_status": "not_run", "source_test_ids": []string{}, "association_basis": "shared proposed Rust module; candidate regressions, not measured symbol coverage", "verification_group_id": id}
			}
		}
	}
	for _, test := range g.inv.Records["tests"] {
		mod := test["rust_module"].(string)
		group := groups[mod]
		group["source_test_ids"] = append(group["source_test_ids"].([]string), test["id"].(string))
	}
	if group := groups["sdk::tools::memory"]; group != nil {
		for _, test := range g.inv.Records["tests"] {
			if strings.HasPrefix(test["source"].(string), "examples/features/memory/") {
				group["source_test_ids"] = append(group["source_test_ids"].([]string), test["id"].(string))
			}
		}
		group["association_basis"] = "memory example regressions include tool-backed custom store"
	}
	for _, group := range groups {
		tests := group["source_test_ids"].([]string)
		sort.Strings(tests)
		if len(tests) == 0 {
			group["coverage_gap"] = "no directly assigned Go test entry points; requires explicit Rust test or documentation/artifact review"
		}
		g.inv.Records["verification_groups"] = append(g.inv.Records["verification_groups"], group)
	}
	for kind, rows := range g.inv.Records {
		sort.Slice(rows, func(i, j int) bool { return rows[i]["id"].(string) < rows[j]["id"].(string) })
		g.inv.Counts[kind] = len(rows)
	}
	return g
}
func main() {
	root := flag.String("sdk", "repos/sdk", "read-only pinned SDK checkout")
	out := flag.String("out", "docs/migration/ledger/sdk-v0.0.115", "generated output directory")
	routes := flag.String("routes", "scripts/inventory/routes.json", "explicit package/artifact ownership")
	check := flag.Bool("check", false, "compare generated bytes without writing")
	flag.Parse()
	g := generate(*root, *routes)
	outputs := map[string][]byte{"inventory.json": writeJSON(g.inv), "sources.json": writeJSON(Row{"schema_version": 1, "sdk_version": "v0.0.115", "commit": pin, "source_license": g.inv.License, "files": g.sources})}
	public := map[string]int{}
	aliases := 0
	linkedValues := 0
	names := map[string]bool{}
	dynamic := []string{}
	for _, r := range g.inv.Records["apis"] {
		if r["visibility"] == "public-package" && r["exported"] == true {
			public[r["kind"].(string)]++
		}
		if r["alias"] == true {
			aliases++
		}
		if r["kind"] == "var" && r["target_id"] != nil {
			linkedValues++
		}
	}
	for _, r := range g.inv.Records["tools"] {
		for _, name := range r["literal_names"].([]string) {
			names[name] = true
		}
		if len(r["literal_schemas"].([]json.RawMessage)) == 0 {
			dynamic = append(dynamic, r["type"].(string))
		}
	}
	generatorSource, err := os.ReadFile(filepath.Join(filepath.Dir(*routes), "main.go"))
	fail(err)
	routeSource, err := os.ReadFile(*routes)
	fail(err)
	outputs["manifest.json"] = writeJSON(Row{"schema_version": 1, "sdk_version": "v0.0.115", "commit": pin, "generator_sha256": hash(string(generatorSource)), "routes_sha256": hash(string(routeSource)), "artifact_sha256": Row{"inventory.json": hash(string(outputs["inventory.json"])), "sources.json": hash(string(outputs["sources.json"]))}, "counts": g.inv.Counts, "tracked_files": len(g.sources), "parsed_go_files": len(g.files), "public_exported_declaration_counts": public, "resolved_type_aliases": aliases, "linked_exported_variables": linkedValues, "distinct_literal_tool_names": len(names), "tool_types_without_literal_schema": dynamic, "unmapped_records": 0, "parse_failures": 0, "source_verification": "git pin and tag, clean checkout, all tracked files parsed or archived; not SDK test execution"})

	for name, data := range outputs {
		p := filepath.Join(*out, name)
		if *check {
			old, err := os.ReadFile(p)
			fail(err)
			if !bytes.Equal(old, data) {
				panic("generated artifact differs: " + p)
			}
		} else {
			fail(os.MkdirAll(*out, 0755))
			fail(os.WriteFile(p, data, 0644))
		}
	}
	fmt.Println(string(writeJSON(g.inv.Counts)))
}
