// SPDX-License-Identifier: GPL-3.0-only
package providers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"sort"
	"strings"
	"testing"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
)

func TestNativeProviderConfigReference(t *testing.T) {
	inputs := []string{
		`{}`,
		`{"provider":"local"}`,
		`{"provider":"copilot"}`,
		`{"provider":"multi","default_provider":"copilot","api_key":"fixture-copilot"}`,
		`{"provider":"anthropic","api_key":"parent","routes":[{"provider":"copilot","prefix":"team","api_key":"fixture-copilot","base_url":"https://custom.example/v1"}]}`,
		`{"provider":" XAI "}`,
		`{"provider":"multi"}`,
		`{"provider":"multi","default_provider":" ANTHROPIC ","model":"openrouter/a/b","api_key":" top ","base_url":" https://example.test/v1 ","auth_mode":"oauth"}`,
		`{"provider":"multi","model":" OpenRouter /anthropic/claude","api_key":" top ","api_mode":"responses"}`,
		`{"provider":" Anthropic ","api_key":" parent ","routes":[{"provider":"groq","prefix":" Team "}]}`,
		`{"provider":"multi","default_provider":"openrouter","api_key":" top ","base_url":"https://top.test/v1","api_mode":"responses","provider_api_keys":{" OPENROUTER ":" override ","anthropic":" secondary "},"provider_base_urls":{" OpenRouter ":" https://override.test/v1 "},"provider_api_modes":{"OPENROUTER":" chat-completions "}}`,
		`{"provider":"multi","default_provider":"openrouter","api_key":" top ","base_url":"https://top.test/v1","api_mode":"responses","provider_api_keys":{" OPENROUTER ":" \t "},"provider_base_urls":{"openrouter":" "},"provider_api_modes":{"openrouter":" "}}`,
		`{"provider":"multi","default_provider":"anthropic","auth_mode":"oauth","provider_auth_modes":{" ANTHROPIC ":" api-key "}}`,
		`{"provider":"multi","default_provider":"openai","provider_auth_modes":{"anthropic":" OAUTH "}}`,
		`{"provider":"multi","default_provider":"openrouter","api_mode":"chat-completions","provider_api_modes":{"openai":"responses"}}`,
		`{"provider":"anthropic","api_key":"parent","base_url":"https://parent.test","auth_mode":"oauth","routes":[{"provider":"anthropic","prefix":"anthropic","api_key":"first"},{"provider":" ANTHROPIC ","prefix":" ","api_key":" last "},{"provider":"anthropic","prefix":"isolated"}]}`,
		`{"routes":[{"provider":"anthropic","prefix":"extra"}]}`,
		`{"provider":"openai","base_url":"https://chatgpt.com/backend-api/codex"}`,
		`{"provider":"multi","model":" /empty","default_provider":" "}`,
	}
	names := map[string]string{"provider": "Provider", "default_provider": "DefaultProvider", "model": "Model", "api_key": "APIKey", "base_url": "BaseURL", "auth_mode": "AuthMode", "api_mode": "APIMode", "provider_api_keys": "ProviderAPIKeys", "provider_base_urls": "ProviderBaseURLs", "provider_api_modes": "ProviderAPIModes", "provider_auth_modes": "ProviderAuthModes", "routes": "Routes", "prefix": "Prefix"}
	var translate func(map[string]any) map[string]any
	translate = func(input map[string]any) map[string]any {
		out := map[string]any{}
		for k, v := range input {
			if k == "routes" {
				var routes []any
				for _, r := range v.([]any) {
					routes = append(routes, translate(r.(map[string]any)))
				}
				v = routes
			}
			out[names[k]] = v
		}
		return out
	}
	observe := func(spec ProviderSpec, kind, prefix string) map[string]any {
		key := apiKeyForProvider(spec, kind)
		base := baseURLForProvider(spec, kind)
		protocol := apiModeForProvider(spec, kind, defaultOpenAICompatibleAPIMode(kind, false))
		auth := "api-key"
		switch kind {
		case "copilot":
			protocol = "copilot"
			auth = "copilot"
			base = firstNonEmpty(base, defaultCopilotBaseURL)
		case "openai":
			protocol = apiModeForProvider(spec, kind, "")
			if base == "" || base == DefaultCodexBackendBaseURL {
				base = "https://api.openai.com/v1"
			}
		case "anthropic":
			protocol = "anthropic"
			if base == "" {
				base = "https://api.anthropic.com"
			}
			if authModeForAnthropicProvider(spec) == "oauth" {
				auth = "oauth"
			}
		default:
			base = firstNonEmpty(base, defaultBaseURLForProvider(kind))
			key = firstNonEmpty(key, defaultAPIKeyForProvider(kind))
		}
		if kind != "anthropic" && kind != "copilot" && strings.ToLower(strings.TrimSpace(protocol)) != "chat-completions" {
			protocol = "responses"
		} else {
			protocol = strings.ToLower(strings.TrimSpace(protocol))
		}
		return map[string]any{"prefix": prefix, "kind": kind, "key": key, "endpoint": base, "protocol": protocol, "auth": auth}
	}
	var cases []any
	for _, input := range inputs {
		var raw map[string]any
		if err := json.Unmarshal([]byte(input), &raw); err != nil {
			t.Fatal(err)
		}
		data, _ := json.Marshal(translate(raw))
		var spec ProviderSpec
		if err := json.Unmarshal(data, &spec); err != nil {
			t.Fatal(err)
		}
		provider, err := NewProviderFromConfig(spec)
		if err != nil {
			t.Fatal(err)
		}
		provider.Close()
		kind := normalizeProviderName(spec.Provider)
		if kind == "" {
			kind = "openai"
		}
		def := kind
		kinds := []string{kind}
		if kind == "multi" || len(spec.Routes) > 0 {
			def = defaultProviderForSpec(spec)
			kinds = []string{"openai", "anthropic", "openrouter", "gemini", "groq", "xai", "local", "copilot"}
		}
		routes := map[string]map[string]any{}
		for _, k := range kinds {
			routes[k] = observe(spec, k, k)
		}
		for _, r := range spec.Routes {
			k := normalizeProviderName(r.Provider)
			p := normalizeProviderName(r.Prefix)
			if p == "" {
				p = k
			}
			routes[p] = observe(specForRoute(r, k), k, p)
		}
		var prefixes []string
		for p := range routes {
			prefixes = append(prefixes, p)
		}
		sort.Strings(prefixes)
		var resolved []any
		for _, p := range prefixes {
			resolved = append(resolved, routes[p])
		}
		cases = append(cases, map[string]any{"input": raw, "default": def, "routes": resolved})
	}
	var wire []any
	for _, flavor := range []string{"key", "bearer", "openrouter"} {
		captured := map[string]any{}
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			var body map[string]any
			if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
				t.Error(err)
			}
			captured["path"] = r.URL.Path
			captured["model"] = body["model"]
			captured["authorization"] = r.Header.Get("Authorization")
			captured["api_key"] = r.Header.Get("X-Api-Key")
			captured["oauth_beta"] = strings.Contains(r.Header.Get("Anthropic-Beta"), "oauth-2025-04-20")
			w.Header().Set("Content-Type", "application/json")
			if flavor == "openrouter" {
				w.Write([]byte(`{"id":"gen-1","model":"anthropic/claude-sonnet-4.6","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}`))
			} else {
				w.Header().Set("Content-Type", "text/event-stream")
				w.Write([]byte("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"))
			}
		}))
		route := ProviderRoute{Provider: "anthropic", Prefix: "selected", APIKey: "fixture-" + flavor, BaseURL: server.URL}
		if flavor == "bearer" {
			route.AuthMode = "oauth"
		}
		name := "selected/claude-sonnet-4-5"
		if flavor == "openrouter" {
			route.Provider = "openrouter"
			route.BaseURL += "/api/v1"
			name = "selected/anthropic/claude-sonnet-4.6"
		}
		p, err := NewProviderFromConfig(ProviderSpec{Provider: "anthropic", APIKey: "parent-not-inherited", Routes: []ProviderRoute{route}})
		if err != nil {
			t.Fatal(err)
		}
		model, err := p.GetModel(name)
		if err != nil {
			t.Fatal(err)
		}
		response, err := model.GetResponse(context.Background(), agentsdk.ModelRequest{Model: strings.TrimPrefix(name, "selected/"), Input: []agentsdk.RunItem{{Type: agentsdk.RunItemMessage, Message: &agentsdk.MessageOutput{Text: "hi"}}}})
		if err != nil {
			t.Fatal(err)
		}
		captured["text"] = response.Items[0].Message.Text
		captured["flavor"] = flavor
		wire = append(wire, captured)
		p.Close()
		server.Close()
	}
	errors := map[string]bool{}
	for name, spec := range map[string]ProviderSpec{"missing": {Routes: []ProviderRoute{{Prefix: "bad"}}}, "nested": {Routes: []ProviderRoute{{Provider: "multi"}}}, "unknown": {Provider: "bogus"}} {
		_, err := NewProviderFromConfig(spec)
		errors[name] = err != nil
	}
	result := map[string]any{"schema_version": 1, "cases": cases, "wire": wire, "errors": errors, "fallbacks": nativeFallbackObservations(t), "copilot": nativeCopilotObservations(t)}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("PROVIDER_CONFIG_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
