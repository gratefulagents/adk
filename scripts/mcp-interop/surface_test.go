package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"sort"
	"strings"
	"testing"
)

// Fail on API drift instead of silently substituting a protocol server for a
// wrapper transport. Read only pristine archived package source, not a mock.
func TestPinnedWrapperServerModeOnlyExportsHTTPHandlerAndClose(t *testing.T) {
	packages, err := parser.ParseDir(token.NewFileSet(), "../../pkg/agentsdk/mcp", func(f os.FileInfo) bool { return !strings.HasSuffix(f.Name(), "_test.go") }, 0)
	if err != nil {
		t.Fatal(err)
	}
	var methods []string
	foundHTTP := false
	for _, f := range packages["mcp"].Files {
		ast.Inspect(f, func(n ast.Node) bool {
			fn, ok := n.(*ast.FuncDecl)
			if !ok {
				return true
			}
			if fn.Recv != nil && len(fn.Recv.List) == 1 {
				if ptr, ok := fn.Recv.List[0].Type.(*ast.StarExpr); ok {
					if typ, ok := ptr.X.(*ast.Ident); ok && typ.Name == "ServerMode" && fn.Name.IsExported() {
						methods = append(methods, fn.Name.Name)
					}
				}
			}
			if fn.Name.Name == "NewServerMode" {
				ast.Inspect(fn.Body, func(n ast.Node) bool {
					if s, ok := n.(*ast.SelectorExpr); ok && s.Sel.Name == "NewStreamableHTTPHandler" {
						foundHTTP = true
					}
					return true
				})
			}
			return true
		})
	}
	sort.Strings(methods)
	if strings.Join(methods, ",") != "Close,Handler" || !foundHTTP {
		t.Fatalf("unexpected wrapper surface: %v HTTP=%v", methods, foundHTTP)
	}
	t.Logf("archived wrapper ServerMode exported methods=%v; NewServerMode constructs NewStreamableHTTPHandler; no exported stdio/SSE server entrypoint", methods)
}
