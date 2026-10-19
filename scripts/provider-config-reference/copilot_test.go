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

func nativeCopilotObservations(t *testing.T) []any {
	var observations []any
	for _, modelName := range []string{"gpt-4.1", "gpt-5.2", "claude-opus-4.8"} {
		reply := `{"id":"fixture","model":"gpt-4.1","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}`
		contentType := "application/json"
		if modelName == "claude-opus-4.8" {
			reply = `{"id":"msg_1","type":"message","role":"assistant","model":"claude-opus-4.8","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}`
		}
		if modelName == "gpt-5.2" {
			contentType = "text/event-stream"
			reply = ""
			for _, event := range []string{
				`{"type":"response.created","response":{"id":"resp_1","model":"gpt-5.2","usage":{"input_tokens":1}}}`,
				`{"type":"response.content_part.added","output_index":0,"part":{"type":"output_text"}}`,
				`{"type":"response.output_text.delta","output_index":0,"delta":"ok"}`,
				`{"type":"response.output_text.done","output_index":0}`,
				`{"type":"response.completed","response":{"id":"resp_1","model":"gpt-5.2","usage":{"input_tokens":1,"output_tokens":1},"output":[]}}`,
			} {
				reply += "data: " + event + "\n\n"
			}
			reply += "data: [DONE]\n\n"
		}
		captured := map[string]any{"input_model": modelName, "reply": reply, "content_type": contentType}
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			var body map[string]any
			if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
				t.Error(err)
			}
			captured["path"] = r.URL.Path
			captured["model"] = body["model"]
			captured["authorization"] = r.Header.Get("Authorization")
			captured["integration"] = r.Header.Get("Copilot-Integration-Id")
			captured["api_version"] = r.Header.Get("X-GitHub-Api-Version")
			w.Header().Set("Content-Type", contentType)
			w.Write([]byte(reply))
		}))
		provider, err := NewProviderFromConfig(ProviderSpec{Provider: "anthropic", APIKey: "parent-must-not-inherit", Routes: []ProviderRoute{{Provider: "copilot", Prefix: "team", APIKey: "fixture-copilot", BaseURL: server.URL + "/v1"}}})
		if err != nil {
			t.Fatal(err)
		}
		model, err := provider.GetModel("team/" + modelName)
		if err != nil {
			t.Fatal(err)
		}
		_, err = model.GetResponse(context.Background(), agentsdk.ModelRequest{Model: modelName, Input: []agentsdk.RunItem{{Type: agentsdk.RunItemMessage, Message: &agentsdk.MessageOutput{Text: "hi"}}}})
		if err != nil {
			t.Fatal(err)
		}
		observations = append(observations, captured)
		provider.Close()
		server.Close()
	}
	return observations
}
