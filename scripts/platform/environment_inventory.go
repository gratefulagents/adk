package main

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

type row map[string]any

type source struct {
	path string
	data []byte
	file *ast.File
}

var envWord = regexp.MustCompile(`(?i)(env|environment)`)
var metadataKey = regexp.MustCompile(`^[a-z0-9.-]+\.(dev|io|org)/[a-zA-Z0-9_.-]+$`)

var keyWord = regexp.MustCompile(`^[A-Z][A-Z0-9_]+$`)

func text(fs *token.FileSet, data []byte, n ast.Node) string {
	return string(data[fs.Position(n.Pos()).Offset:fs.Position(n.End()).Offset])
}

func callName(e ast.Expr) string {
	switch n := e.(type) {
	case *ast.Ident:
		return n.Name
	case *ast.SelectorExpr:
		return callName(n.X) + "." + n.Sel.Name
	}
	return ""
}

func classify(fs *token.FileSet, data []byte, n ast.Node, helpers map[string]bool) string {
	switch n := n.(type) {
	case *ast.FuncDecl:
		if envWord.MatchString(n.Name.Name) {
			return "environment-helper-definition"
		}
	case *ast.CallExpr:
		name := callName(n.Fun)
		switch name {
		case "os.Getenv", "os.LookupEnv", "os.Environ", "os.ExpandEnv", "syscall.Getenv", "syscall.Environ":
			return "consumer"
		case "os.Setenv", "os.Unsetenv", "os.Clearenv", "syscall.Setenv", "syscall.Unsetenv", "t.Setenv", "b.Setenv":
			return "producer"
		}
		parts := strings.Split(name, ".")
		if envWord.MatchString(name) || helpers[parts[len(parts)-1]] {
			return "helper-or-forwarding-expression"
		}
	case *ast.CompositeLit:
		typ := ""
		if n.Type != nil {
			typ = text(fs, data, n.Type)
		}
		keys := map[string]bool{}
		for _, elt := range n.Elts {
			if kv, ok := elt.(*ast.KeyValueExpr); ok {
				keys[text(fs, data, kv.Key)] = true
			}
		}
		if strings.Contains(typ, "EnvVar") || (keys["Name"] && (keys["Value"] || keys["ValueFrom"])) {
			return "kubernetes-producer-expression"
		}
	case *ast.KeyValueExpr:
		if envWord.MatchString(text(fs, data, n.Key)) {
			return "environment-field-or-map-expression"
		}
	case *ast.AssignStmt:
		for _, lhs := range n.Lhs {
			if envWord.MatchString(text(fs, data, lhs)) {
				return "environment-assignment-expression"
			}
		}
	case *ast.ValueSpec:
		for _, name := range n.Names {
			if envWord.MatchString(name.Name) {
				return "environment-definition"
			}
		}
	case *ast.RangeStmt:
		if envWord.MatchString(text(fs, data, n.X)) {
			return "environment-forwarding-loop"
		}
	}
	return ""
}

func main() {
	fs := token.NewFileSet()
	var sources []source
	var scanned []string
	sourceHashes := map[string]string{}
	goImports := map[string][]string{}
	var metadata []row
	var registration []row
	var nonGo []row
	for _, root := range []string{"repos/gratefulagents", "repos/sdk"} {
		pins := map[string]string{"repos/gratefulagents": "08e65c970830f05042c251bcbb46ec6a9e3719b9", "repos/sdk": "1dc92b73900fac74dc357a938e4b5eee6392b418"}
		head, err := exec.Command("git", "-C", root, "rev-parse", "HEAD").Output()
		if err != nil || strings.TrimSpace(string(head)) != pins[root] {
			panic("source pin mismatch: " + root)
		}
		status, err := exec.Command("git", "-C", root, "status", "--porcelain").Output()
		if err != nil || len(status) != 0 {
			panic("dirty source repository: " + root)
		}
		out, err := exec.Command("git", "-C", root, "ls-files").Output()
		if err != nil {
			panic(err)
		}
		for _, relative := range strings.Split(strings.TrimSpace(string(out)), "\n") {
			path := filepath.ToSlash(filepath.Join(root, relative))
			if !strings.HasSuffix(path, ".go") {
				ext := filepath.Ext(path)
				if ext == ".yaml" || ext == ".yml" || ext == ".sh" || strings.Contains(filepath.Base(path), "Dockerfile") {
					data, err := os.ReadFile(path)
					if err != nil {
						panic(err)
					}
					lines := strings.Split(string(data), "\n")
					for i, line := range lines {
						if envWord.MatchString(line) || strings.Contains(line, "$") || strings.Contains(line, "- name:") || strings.Contains(line, "export ") {
							start, end := max(0, i-2), min(len(lines), i+4)
							nonGo = append(nonGo, row{"source": fmt.Sprintf("%s:%d", path, i+1), "expression": line, "context": strings.Join(lines[start:end], "\n"), "classification": "deployment-or-shell-candidate"})
						}
					}
				}
				continue
			}
			data, err := os.ReadFile(path)
			if err != nil {
				panic(err)
			}
			file, err := parser.ParseFile(fs, path, data, 0)
			if err != nil {
				panic(err)
			}
			sourceHashes[path] = fmt.Sprintf("%x", sha256.Sum256(data))
			goImports[path] = []string{}
			for _, imp := range file.Imports {
				name, err := strconv.Unquote(imp.Path.Value)
				if err != nil {
					panic(err)
				}
				goImports[path] = append(goImports[path], name)
			}
			sources = append(sources, source{path, data, file})
			scanned = append(scanned, path)
		}
	}
	helperNames := map[string]bool{}
	for _, s := range sources {
		for _, decl := range s.file.Decls {
			fn, ok := decl.(*ast.FuncDecl)
			if !ok {
				continue
			}
			if envWord.MatchString(fn.Name.Name) {
				helperNames[fn.Name.Name] = true
			}
			ast.Inspect(fn, func(n ast.Node) bool {
				call, ok := n.(*ast.CallExpr)
				if ok && (callName(call.Fun) == "os.Getenv" || callName(call.Fun) == "os.LookupEnv") && len(call.Args) > 0 {
					if _, literal := call.Args[0].(*ast.BasicLit); !literal {
						helperNames[fn.Name.Name] = true
					}
				}
				return true
			})
		}
	}
	var records []row
	contexts := map[string]row{}
	constants := map[string]string{}
	index := map[string][]string{}
	for _, s := range sources {
		for _, decl := range s.file.Decls {
			ast.Inspect(decl, func(n ast.Node) bool {
				if value, ok := n.(*ast.ValueSpec); ok {
					for i, name := range value.Names {
						if i < len(value.Values) {
							if lit, ok := value.Values[i].(*ast.BasicLit); ok && lit.Kind == token.STRING {
								v, err := strconv.Unquote(lit.Value)
								if err == nil && keyWord.MatchString(v) {
									constants[filepath.Dir(s.path)+"::"+name.Name] = v
								}
							}
						}
					}
				}
				if n == nil {
					return true
				}
				position := fs.Position(n.Pos())
				witness := row{"source": s.path, "line": position.Line, "end_line": fs.Position(n.End()).Line, "expression": text(fs, s.data, n), "test": strings.HasSuffix(s.path, "_test.go")}
				if call, ok := n.(*ast.CallExpr); ok {
					name := callName(call.Fun)
					if strings.Contains(name, "Register") || strings.Contains(name, "Registry") || strings.HasPrefix(name, "tools.With") || strings.HasPrefix(name, "sdktools.With") || name == "setupAskTeammateTool" || strings.Contains(name, "buildMCPConfig") {
						registration = append(registration, witness)
					}
				}
				isMetadata := false
				if lit, ok := n.(*ast.BasicLit); ok && lit.Kind == token.STRING {
					value, err := strconv.Unquote(lit.Value)
					if err == nil && metadataKey.MatchString(value) {
						isMetadata = true
						witness["literal_key"] = value
					}
				}
				if expr, ok := n.(*ast.IndexExpr); ok {
					target := text(fs, s.data, expr.X)
					if strings.Contains(target, "Annotations") || strings.Contains(target, "Labels") {
						isMetadata = true
					}
				}
				if value, ok := n.(*ast.ValueSpec); ok {
					for _, name := range value.Names {
						if strings.Contains(name.Name, "Annotation") || strings.Contains(name.Name, "Label") {
							isMetadata = true
						}
					}
				}
				if isMetadata {
					metadata = append(metadata, witness)
				}
				kind := classify(fs, s.data, n, helperNames)
				if kind == "" {
					return true
				}
				pos := fs.Position(n.Pos())
				id := fmt.Sprintf("%s:%d:%d", s.path, pos.Line, pos.Column)
				contextID := fmt.Sprintf("%s:%d", s.path, fs.Position(decl.Pos()).Line)
				contexts[contextID] = row{"source": contextID, "expression": text(fs, s.data, decl)}
				keys := []string{}
				ast.Inspect(n, func(child ast.Node) bool {
					if lit, ok := child.(*ast.BasicLit); ok && lit.Kind == token.STRING {
						v, err := strconv.Unquote(lit.Value)
						if err == nil && keyWord.MatchString(v) {
							keys = append(keys, v)
							index[v] = append(index[v], id)
						}
					}
					return true
				})
				records = append(records, row{"id": id, "source": s.path, "line": pos.Line, "end_line": fs.Position(n.End()).Line, "classification": kind, "expression": text(fs, s.data, n), "context_id": contextID, "literal_key_candidates": keys, "test": strings.HasSuffix(s.path, "_test.go")})
				return true
			})
		}
	}
	sort.Strings(scanned)
	result := row{"format": "platform-environment-expressions/v1", "scope": "All tracked Go files in both pinned repositories, including tests and all build tags; deployment/shell text candidates supplementary", "method": "Go AST direct process reads/writes; Env-named helpers/fields/definitions/assignments/loops; dynamic os.Getenv/LookupEnv wrappers and their name-matched call sites; Kubernetes EnvVar and inferred Name+Value/ValueFrom literals", "limitations": []string{"Static expression inventory, not a runtime value enumeration or whole-program dataflow proof.", "Helper name matching and Env substring candidates deliberately over-approximate; some rows are not process environment operations.", "Full enclosing declarations retain dynamic provider switches, MCP names, sandbox constants, forwarding loops and defaults; uppercase literals are candidates, not necessarily environment names.", "Third-party library implicit reads are outside the local source inventory; non-Go rows are textual candidates, not a parsed deployment manifest ABI."}, "counts": row{"go_files_scanned": len(scanned), "expression_records": len(records), "contexts": len(contexts), "literal_key_candidates": len(index), "non_go_candidates": len(nonGo)}, "scanned_files": scanned, "records": records, "contexts": contexts, "constant_key_candidates": constants, "literal_key_index": index, "deployment_shell_candidates": nonGo}
	result["source_sha256"] = sourceHashes
	result["go_imports"] = goImports
	result["metadata_key_and_access_expressions"] = metadata
	result["registration_expressions"] = registration
	result["counts"].(row)["metadata_expressions"] = len(metadata)
	result["counts"].(row)["registration_expressions"] = len(registration)
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(result); err != nil {
		panic(err)
	}
}
