// SPDX-License-Identifier: GPL-3.0-only
package providers

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	sdkopenai "github.com/gratefulagents/sdk/pkg/agentsdk/providers/openai"
)

func nativeOpenAiOAuthObservation(t *testing.T) map[string]any {
	reply := ""
	for _, event := range []string{
		`{"type":"response.created","response":{"id":"resp_fixture","model":"gpt-5.2"}}`,
		`{"type":"response.content_part.added","output_index":0,"part":{"type":"output_text"}}`,
		`{"type":"response.output_text.delta","output_index":0,"delta":"ok"}`,
		`{"type":"response.output_text.done","output_index":0}`,
		`{"type":"response.completed","response":{"id":"resp_fixture","model":"gpt-5.2","output":[],"usage":{"input_tokens":1,"output_tokens":1}}}`,
	} {
		reply += "data: " + event + "\n\n"
	}
	reply += "data: [DONE]\n\n"
	captured := map[string]any{"reply": reply}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		captured["path"] = r.URL.Path
		captured["model"] = body["model"]
		captured["authorization"] = r.Header.Get("Authorization")
		captured["account"] = r.Header.Get("Chatgpt-Account-Id")
		w.Header().Set("Content-Type", "text/event-stream")
		w.Write([]byte(reply))
	}))
	defer server.Close()
	session, err := sdkopenai.NewOAuthAuthSessionFromSecretData([]byte(`{"tokens":{"access_token":"fixture-openai-oauth","account_id":"selected-account"}}`), "selected-account")
	if err != nil {
		t.Fatal(err)
	}
	provider, err := NewProviderFromConfig(ProviderSpec{Provider: "openai", AuthMode: "oauth", OpenAIOAuthAccountID: "parent-account", Routes: []ProviderRoute{{Provider: "openai", Prefix: "selected", AuthMode: "oauth", BaseURL: server.URL + "/v1", OpenAIOAuthAccountID: "selected-account", OpenAIAuthSession: session}}})
	if err != nil {
		t.Fatal(err)
	}
	defer provider.Close()
	model, err := provider.GetModel("selected/gpt-5.2")
	if err != nil {
		t.Fatal(err)
	}
	_, err = model.GetResponse(context.Background(), agentsdk.ModelRequest{Model: "gpt-5.2", Input: []agentsdk.RunItem{{Type: agentsdk.RunItemMessage, Message: &agentsdk.MessageOutput{Text: "hi"}}}})
	if err != nil {
		t.Fatal(err)
	}
	return captured
}
