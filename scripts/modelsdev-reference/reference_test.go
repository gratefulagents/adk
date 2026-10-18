package modelsdev

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"
	"time"
	"unicode"
)

func TestNativeModelsDevReference(t *testing.T) {
	inputs := []string{sampleCatalog, `{}`, `null`, `[]`, `not json`, `{"p":null}`, `{"p":{"models":null}}`}
	for _, limit := range []string{`null`, `{}`, `{"context":1}`, `{"context":-1}`, `{"context":0}`, `{"context":-0}`, `{"context":1.0}`, `{"context":1e3}`, `{"context":9223372036854775807,"output":-9223372036854775808}`, `{"context":9223372036854775808}`, `{"context":true}`, `{"CONTEXT":99,"output":-7}`, `{"context":99,"context":null}`, `{"context":99,"output":4,"OUTPUT":null}`} {
		inputs = append(inputs, `{" P ":{"MODELS":{" M ":{"LIMIT":`+limit+`},"": {"limit":{"context":3}}}}}`)
	}
	inputs = append(inputs, `{"p":{"models":{"m":{"limit":{"context":8,"output":3},"LIMIT":{"context":null,"output":5}}}}}`, `{"p":{"models":{"m":{"limit":{"context":8}}},"MODELS":{"n":{"limit":{"context":16}}}}}`, `{"p":{"models":{"m":{"limit":{"context":8}},"m":{"limit":{"output":3}}}}}`, `{"p":{"models":{"m":{"limit":{"context":8}}},"models":null}}`, `{"p":{"models":{"m":{"limit":{"context":8}}}},"p":{"models":{"n":{"limit":{"context":16}}}}}`)
	inputs = append(inputs, `{"\ud800":{"models":{"\udc00":{"limit":{"context":3}}}}}`, `{"p":{"modelſ":{"m":{"limit":{"context":3}}}}}`)
	for _, depth := range []int{128, 9999, 10000} {
		inputs = append(inputs, `{"ignored":`+strings.Repeat("[", depth)+`0`+strings.Repeat("]", depth)+`,"p":{"models":{"m":{"limit":{"context":3}}}}}`)
	}
	parsed := []map[string]any{}
	for _, input := range inputs {
		catalog, err := ParseCatalog([]byte(input))
		parsed = append(parsed, map[string]any{"input": input, "ok": err == nil, "catalog": catalog})
	}
	catalog, err := ParseCatalog([]byte(sampleCatalog))
	if err != nil {
		t.Fatal(err)
	}
	catalog["openai"]["overflow"] = ModelLimits{ContextTokens: 9223372036854775807, OutputTokens: -1}
	r := NewResolver(WithCachePath(""))
	r.catalog = catalog
	r.fetchedAt = time.Now()
	resolved := []map[string]any{}
	for _, provider := range []string{"copilot", "github-copilot", " COPILOT ", "openai", "openai-oauth", "anthropic", "anthropic-oauth", "unknown", ""} {
		for _, model := range []string{"claude-fable-5", "copilot/claude-fable-5", " CLAUDE-FABLE-5 ", "claude-fable-5-20270101", "claude-fable", "gpt-5.3-codex", "gpt-5.3-codex-spark", "openai/gpt-5.4", "openai-oauth/gpt-5.4", "gpt-5.6-sol", "overflow", "", "unknown", "/gpt-5.4"} {
			limits, ok := r.Lookup(context.Background(), provider, model)
			trigger, target, thresholdOK := r.CompactionThresholds(context.Background(), provider, model)
			rt, rg, ro := r.CompactionResolverFunc(provider)(context.Background(), model)
			resolved = append(resolved, map[string]any{"provider": provider, "model": model, "limits": limits, "ok": ok, "thresholds": []any{trigger, target, thresholdOK}, "routed": []any{rt, rg, ro}})
		}
	}
	lower := map[string]string{}
	for c := rune(0); c <= unicode.MaxRune; c++ {
		if c >= 0xd800 && c <= 0xdfff {
			continue
		}
		v := string(c)
		if strings.ToLower(v) != v {
			lower[v] = strings.ToLower(v)
		}
	}
	result := map[string]any{"parsed": parsed, "catalog": catalog, "resolved": resolved, "lower": lower}
	data, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("MODELSDEV_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
