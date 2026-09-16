package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"testing"
)

func TestEnvironmentExpressionCoverage(t *testing.T) {
	cases := []struct{ expression, want string }{
		{`os.Getenv(providerAPIKeyEnvName(provider))`, "consumer"},
		{`os.LookupEnv(sandbox.SandboxModeEnv)`, "consumer"},
		{`os.Environ()`, "consumer"},
		{`os.Setenv(name, value)`, "producer"},
		{`os.Unsetenv("TOKEN")`, "producer"},
		{`agentinfra.MustEnv("MODEL")`, "helper-or-forwarding-expression"},
		{`readSetting("WORKSPACE_DIR")`, "helper-or-forwarding-expression"},
		{`corev1.EnvVar{Name: name, ValueFrom: secret}`, "kubernetes-producer-expression"},
		{`corev1.EnvVar{Name: sandbox.SandboxModeEnv, Value: "required"}`, "kubernetes-producer-expression"},
		{`strings.TrimSpace(value)`, ""},
	}
	for _, tc := range cases {
		t.Run(tc.expression, func(t *testing.T) {
			fs := token.NewFileSet()
			x, err := parser.ParseExprFrom(fs, "test.go", tc.expression, 0)
			if err != nil {
				t.Fatal(err)
			}
			if got := classify(fs, []byte(tc.expression), x, map[string]bool{"readSetting": true}); got != tc.want {
				t.Fatalf("got %q; want %q", got, tc.want)
			}
			if got := text(fs, []byte(tc.expression), x); got != tc.expression {
				t.Fatalf("lost expression: %s", got)
			}
		})
	}
}

func TestInferredKubernetesEnvironmentAndDynamicDefinition(t *testing.T) {
	code := []byte(`package p
var vars = []corev1.EnvVar{{Name: "MODE_MAX_TURNS", Value: value}}
func providerAPIKeyEnvName(provider string) string { return strings.ToUpper(provider) + "_API_KEY" }
func launch() { cmd.Env = append(os.Environ(), extra...); for _, name := range sandbox.SandboxConfigEnvNames() { use(name) } }
`)
	fs := token.NewFileSet()
	f, err := parser.ParseFile(fs, "test.go", code, 0)
	if err != nil {
		t.Fatal(err)
	}
	seen := map[string]int{}
	ast.Inspect(f, func(n ast.Node) bool { seen[classify(fs, code, n, nil)]++; return true })
	for _, key := range []string{"kubernetes-producer-expression", "environment-helper-definition", "environment-assignment-expression", "environment-forwarding-loop"} {
		if seen[key] == 0 {
			t.Errorf("missing %s", key)
		}
	}
}
