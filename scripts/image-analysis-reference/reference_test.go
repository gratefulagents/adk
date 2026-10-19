// SPDX-License-Identifier: GPL-3.0-only
package openai

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
)

func TestNativeImageAnalysisReference(t *testing.T) {
	cases := []struct {
		Name    string `json:"name"`
		Model   string `json:"model"`
		Data    string `json:"data"`
		MIME    string `json:"mime"`
		Prompt  string `json:"prompt"`
		Detail  string `json:"detail"`
		Default bool   `json:"default"`
		NoText  bool   `json:"no_text"`
	}{
		{Name: "default", Data: "hello", MIME: "image/png", Prompt: "what is shown?", Default: true},
		{Name: "low", Model: " gpt-5.5 ", Data: "hello", MIME: " image/png ; charset=binary", Prompt: " what is shown? ", Detail: " LOW "},
		{Name: "original", Model: "gpt-5.5", Data: "hello", MIME: "image/jpeg", Prompt: "describe", Detail: "original"},
		{Name: "auto", Data: "hello", MIME: "image/webp", Prompt: "describe", Detail: "auto"},
		{Name: "unknown_detail", Data: "hello", MIME: "image/gif", Prompt: "describe", Detail: "other"},
		{Name: "empty_mime", Data: "hello", Prompt: "describe"},
		{Name: "invalid_mime", Data: "hello", MIME: "image/ png", Prompt: "describe"},
		{Name: "comma_mime", Data: "hello", MIME: "image/png,bad", Prompt: "describe"},
		{Name: "empty_data", MIME: "image/png", Prompt: "describe"},
		{Name: "empty_prompt", Data: "hello", MIME: "image/png", Prompt: " \t "},
		{Name: "no_text", Data: "hello", MIME: "image/png", Prompt: "describe", NoText: true},
	}
	observations := []map[string]any{}
	for _, c := range cases {
		t.Run(c.Name, func(t *testing.T) {
			var captured map[string]any
			var path string
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				path = r.URL.Path
				if r.Header.Get("Authorization") != "Bearer fixture-token" {
					t.Error("unexpected authorization")
				}
				if err := json.NewDecoder(r.Body).Decode(&captured); err != nil {
					t.Error(err)
				}
				w.Header().Set("Content-Type", "text/event-stream")
				fmt.Fprint(w, "data: {\"type\":\"response.created\",\"response\":{\"id\":\"vision\",\"model\":\"gpt-5.5\"}}\n\n")
				if !c.NoText {
					fmt.Fprint(w, "data: {\"type\":\"response.content_part.added\",\"output_index\":0,\"part\":{\"type\":\"output_text\"}}\n\n")
					fmt.Fprint(w, "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"  vision ok  \"}\n\n")
					fmt.Fprint(w, "data: {\"type\":\"response.output_text.done\",\"output_index\":0}\n\n")
				}
				fmt.Fprint(w, "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"vision\",\"model\":\"gpt-5.5\",\"output\":[]}}\n\ndata: [DONE]\n\n")
			}))
			defer server.Close()
			model, err := newOpenAIModelFromConfig(server.URL+"/v1", "", "chat", "fixture-token", nil, nil)
			if err != nil {
				t.Fatal(err)
			}
			model.model = c.Model
			var result string
			if c.Default {
				result, err = model.AnalyzeImage(context.Background(), []byte(c.Data), c.MIME, c.Prompt)
			} else {
				result, err = model.AnalyzeImageWithDetail(context.Background(), []byte(c.Data), c.MIME, c.Prompt, c.Detail)
			}
			errorText := ""
			if err != nil {
				errorText = err.Error()
			}
			observations = append(observations, map[string]any{"input": c, "request": captured, "path": path, "result": result, "error": errorText})
		})
	}
	data, err := json.MarshalIndent(map[string]any{"cases": observations}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
