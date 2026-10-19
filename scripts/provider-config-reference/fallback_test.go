// SPDX-License-Identifier: GPL-3.0-only
package providers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
)

func nativeFallbackObservations(t *testing.T) []any {
	var observations []any
	for _, kind := range []string{"openrouter", "groq", "named"} {
		for _, fallbacks := range [][]string{nil, {" primary ", " backup ", "", "backup", "other"}, {"primary", " "}} {
			var body map[string]any
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
					t.Error(err)
				}
				w.Header().Set("Content-Type", "application/json")
				w.Write([]byte(`{"id":"fixture","model":"primary","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}`))
			}))
			spec := ProviderSpec{Provider: kind, APIKey: "fixture", BaseURL: server.URL, APIMode: "chat-completions", ModelFallbacks: fallbacks}
			name := "primary"
			if kind == "named" {
				spec.Provider = "openrouter"
				spec.Routes = []ProviderRoute{{Provider: "openrouter", Prefix: "named", APIKey: "fixture", BaseURL: server.URL}}
				name = "named/primary"
			}
			provider, err := NewProviderFromConfig(spec)
			if err != nil {
				t.Fatal(err)
			}
			model, err := provider.GetModel(name)
			if err != nil {
				t.Fatal(err)
			}
			_, err = model.GetResponse(context.Background(), agentsdk.ModelRequest{Model: "primary", Input: []agentsdk.RunItem{{Type: agentsdk.RunItemMessage, Message: &agentsdk.MessageOutput{Text: "hi"}}}})
			if err != nil {
				t.Fatal(err)
			}
			// Normalize nil only for the input vector; wire omission is observed separately.
			if fallbacks == nil {
				fallbacks = []string{}
			}
			models, present := body["models"]
			observations = append(observations, map[string]any{"kind": kind, "configured": fallbacks, "models": models, "present": present})
			provider.Close()
			server.Close()
		}
	}
	return observations
}
