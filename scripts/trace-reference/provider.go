package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"time"

	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/providers/anthropic"
	"github.com/gratefulagents/sdk/pkg/agentsdk/providers/openai"
)

type providerResponseCase struct {
	Protocol           string   `json:"protocol"`
	Method             string   `json:"method"`
	Body               string   `json:"body"`
	RawJSON            string   `json:"raw_json"`
	SnapshotJSON       string   `json:"snapshot_json"`
	StreamRawJSON      string   `json:"stream_raw_json,omitempty"`
	StreamSnapshotJSON string   `json:"stream_snapshot_json,omitempty"`
	Streaming          bool     `json:"streaming,omitempty"`
	Events             []string `json:"events,omitempty"`
}

func providerResponseCases() map[string]providerResponseCase {
	cases := map[string]providerResponseCase{
		"responses_text_reasoning_tool": {
			Method:    "complete",
			Protocol:  "responses",
			Body:      `{"id":"resp_full","model":"fixture-model","status":"completed","output":[{"type":"message","id":"msg_full","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"answer <>&"}]},{"type":"reasoning","id":"reason_full","summary":[{"type":"summary_text","text":"first "},{"type":"summary_text","text":"second"}],"encrypted_content":"opaque-reason"},{"type":"function_call","id":"fc_full","call_id":"call_full","name":"lookup","arguments":"{ \"z\": 1e+02, \"a\": {\"two\":2.00,\"one\":-0}, \"q\":\"<>&\" }"}],"usage":{"input_tokens":21,"output_tokens":8,"input_tokens_details":{"cached_tokens":5,"cache_write_tokens":3}},"provider_extra":"not retained"}`,
			Streaming: true,
			Events: []string{
				"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_full\",\"model\":\"fixture-model\"}}\n\n",
				"event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_full\",\"role\":\"assistant\",\"phase\":\"commentary\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer <>&\"}]}}\n\n",
				"event: response.content_part.added\ndata: {\"type\":\"response.content_part.added\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\"}}\n\n",
				"event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"output_index\":0,\"text\":\"answer <>&\"}\n\n",
				"event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_full\",\"role\":\"assistant\",\"phase\":\"commentary\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer <>&\"}]}}\n\n",
				"event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"reasoning\",\"id\":\"reason_full\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"first \"},{\"type\":\"summary_text\",\"text\":\"second\"}],\"encrypted_content\":\"opaque-reason\"}}\n\n",
				"event: response.reasoning_summary_text.delta\ndata: {\"type\":\"response.reasoning_summary_text.delta\",\"output_index\":1,\"delta\":\"first \"}\n\n",
				"event: response.reasoning_summary_text.delta\ndata: {\"type\":\"response.reasoning_summary_text.delta\",\"output_index\":1,\"delta\":\"second\"}\n\n",
				"event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"reasoning\",\"id\":\"reason_full\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"first \"},{\"type\":\"summary_text\",\"text\":\"second\"}],\"encrypted_content\":\"opaque-reason\"}}\n\n",
				"event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":2,\"item\":{\"type\":\"function_call\",\"id\":\"fc_full\",\"call_id\":\"call_full\",\"name\":\"lookup\",\"arguments\":\"{ \\\"z\\\": 1e+02, \\\"a\\\": {\\\"two\\\":2.00,\\\"one\\\":-0}, \\\"q\\\":\\\"<>&\\\" }\"}}\n\n",
				"event: response.function_call_arguments.done\ndata: {\"type\":\"response.function_call_arguments.done\",\"output_index\":2,\"arguments\":\"{ \\\"z\\\": 1e+02, \\\"a\\\": {\\\"two\\\":2.00,\\\"one\\\":-0}, \\\"q\\\":\\\"<>&\\\" }\"}\n\n",
				"event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":2,\"item\":{\"type\":\"function_call\",\"id\":\"fc_full\",\"call_id\":\"call_full\",\"name\":\"lookup\",\"arguments\":\"{ \\\"z\\\": 1e+02, \\\"a\\\": {\\\"two\\\":2.00,\\\"one\\\":-0}, \\\"q\\\":\\\"<>&\\\" }\"}}\n\n",
			},
		},
		"responses_compaction_max_tokens_false": {
			Method:    "complete",
			Protocol:  "responses",
			Body:      `{"id":"resp_compact","model":"fixture-model","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"end_turn":false,"output":[{"type":"compaction","id":"compact_full","encrypted_content":"opaque-compact","created_by":"openai"}],"usage":{"input_tokens":13,"output_tokens":4}}`,
			Streaming: true,
			Events: []string{
				"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_compact\",\"model\":\"fixture-model\"}}\n\n",
				"event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"compaction\",\"id\":\"compact_full\",\"encrypted_content\":\"opaque-compact\",\"created_by\":\"openai\"}}\n\n",
				"event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"compaction\",\"id\":\"compact_full\",\"encrypted_content\":\"opaque-compact\",\"created_by\":\"openai\"}}\n\n",
			},
		},
		"responses_defaults": {
			Method:    "complete",
			Protocol:  "responses",
			Body:      `{"output":[{"type":"message","content":[{"type":"output_text","text":"done"}]}]}`,
			Streaming: true,
			Events: []string{
				"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{}}\n\n",
				"event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"done\"}]}}\n\n",
				"event: response.content_part.added\ndata: {\"type\":\"response.content_part.added\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\"}}\n\n",
				"event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"output_index\":0,\"text\":\"done\"}\n\n",
				"event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"done\"}]}}\n\n",
			},
		},
		"chat_text_reasoning_tool": {
			Method:   "complete",
			Protocol: "chat",
			Body:     `{"id":"chat_full","model":"fixture-model","choices":[{"index":0,"message":{"role":"assistant","content":"answer <>&","reasoning_content":"consider carefully","reasoning_opaque":"opaque-signature","tool_calls":[{"id":"call_full","type":"function","function":{"name":"lookup","arguments":"{ \"z\": 1e+02, \"a\": {\"two\":2.00,\"one\":-0}, \"q\":\"<>&\" }"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":21,"completion_tokens":8},"provider_extra":"not retained"}`,
		},
		"chat_refusal_array_length_cache": {
			Method:   "complete",
			Protocol: "chat",
			Body:     `{"id":"chat_length","model":"fixture-model","choices":[{"message":{"role":"assistant","content":[{"type":"text","text":"first"},{"type":"image_url","image_url":{"url":"https://example.invalid/not-fetched"}},{"type":"text","text":"second"}],"refusal":"not allowed"},"finish_reason":"length"}],"usage":{"prompt_tokens":15,"completion_tokens":6,"prompt_tokens_details":{"cached_tokens":4,"cache_write_tokens":2}}}`,
		},
		"anthropic_text_thinking_redacted_tool": {
			Method:    "complete",
			Protocol:  "anthropic",
			Streaming: true,
			Events: []string{
				"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_full\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"fixture-model\",\"content\":[],\"usage\":{\"input_tokens\":21,\"output_tokens\":8,\"cache_read_input_tokens\":5,\"cache_creation_input_tokens\":3},\"provider_extra\":\"not retained\"}}\n\n",
				"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"answer <>&\"}}\n\n",
				"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
				"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"consider carefully\",\"signature\":\"signed-thinking\"}}\n\n",
				"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
				"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"redacted_thinking\",\"data\":\"redacted-data\"}}\n\n",
				"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
				"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":3,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_full\",\"name\":\"lookup\",\"input\":{}}}\n\n",
				"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":3,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{ \\\"z\\\": 1e+02, \\\"a\\\": {\\\"two\\\":2.00,\\\"one\\\":-0}, \\\"q\\\":\\\"<>&\\\" }\"}}\n\n",
				"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":3}\n\n",
				"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"input_tokens\":21,\"output_tokens\":8,\"cache_read_input_tokens\":5,\"cache_creation_input_tokens\":3}}\n\n",
				"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
			},
		},
		"anthropic_compaction_cache": {
			Method:    "complete",
			Protocol:  "anthropic",
			Streaming: true,
			Events: []string{
				"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_compact\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"fixture-model\",\"content\":[],\"usage\":{\"input_tokens\":13,\"output_tokens\":4,\"cache_read_input_tokens\":6,\"cache_creation_input_tokens\":2}}}\n\n",
				"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"compaction\",\"id\":\"ignored-id\",\"created_by\":\"ignored-origin\",\"content\":\"summary <>&\",\"encrypted_content\":\"opaque-compact\"}}\n\n",
				"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
				"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"input_tokens\":13,\"output_tokens\":4,\"cache_read_input_tokens\":6,\"cache_creation_input_tokens\":2}}\n\n",
				"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
			},
		},
		"anthropic_defaults": {
			Method:    "complete",
			Protocol:  "anthropic",
			Streaming: true,
			Events: []string{
				"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"content\":[]}}\n\n",
				"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"done\"}}\n\n",
				"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
				"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
			},
		},
		"anthropic_stream_text_tool": {
			Method:    "stream",
			Protocol:  "anthropic",
			Streaming: true,
			Events: []string{
				"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"fixture-model\",\"content\":[],\"usage\":{\"input_tokens\":12,\"output_tokens\":0,\"cache_read_input_tokens\":3}}}\n\n",
				"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
				"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"stream <>&\"}}\n\n",
				"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
				"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_stream\",\"name\":\"lookup\",\"input\":{}}}\n\n",
				"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{ \\\"z\\\": 1e+02,\"}}\n\n",
				"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\" \\\"a\\\": 2.00 }\"}}\n\n",
				"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
				"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":5}}\n\n",
				"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
			},
		},
	}
	for name, fixture := range cases {
		func() {
			// The pinned Responses GetResponse path itself consumes SSE.
			if fixture.Protocol == "responses" {
				terminal := "response.completed"
				if name == "responses_compaction_max_tokens_false" {
					terminal = "response.incomplete"
				}
				fixture.Events = append(fixture.Events, "event: "+terminal+"\ndata: {\"type\":\""+terminal+"\",\"response\":"+fixture.Body+"}\n\n")
			}
			if fixture.Streaming {
				fixture.Body = strings.Join(fixture.Events, "")
			}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				path := "/v1/responses"
				switch fixture.Protocol {
				case "chat":
					path = "/v1/chat/completions"
				case "anthropic":
					path = "/v1/messages"
				}
				if r.Method != http.MethodPost || r.URL.Path != path {
					http.Error(w, "unexpected provider endpoint: "+r.Method+" "+r.URL.Path, http.StatusBadRequest)
					return
				}
				w.Header().Set("Content-Type", "application/json")
				if fixture.Streaming {
					w.Header().Set("Content-Type", "text/event-stream")
				}
				_, _ = io.WriteString(w, fixture.Body)
			}))
			defer server.Close()
			var model agent.Model
			var err error
			if fixture.Protocol == "anthropic" {
				provider := anthropic.NewAnthropicProviderWithConfig(anthropic.ProviderConfig{APIKey: "fixture-fake-key", BaseURL: server.URL})
				defer provider.Close()
				model, err = provider.GetModel("fixture-model")
			} else {
				mode := "responses"
				if fixture.Protocol == "chat" {
					mode = "chat-completions"
				}
				provider := openai.NewOpenAIProviderWithConfig(openai.ProviderConfig{APIKey: "fixture-fake-key", BaseURL: server.URL, APIMode: mode})
				defer provider.Close()
				model, err = provider.GetModel("fixture-model")
			}
			must(err)
			ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel()
			request := agent.ModelRequest{Input: []agent.RunItem{{Type: agent.RunItemMessage, Message: &agent.MessageOutput{Text: "fixture prompt"}}}}
			var response *agent.ModelResponse
			if fixture.Method == "stream" {
				stream, err := model.StreamResponse(ctx, request)
				must(err)
				for event := range stream.Events {
					must(event.Error)
				}
				response = stream.Final()
			} else {
				response, err = model.GetResponse(ctx, request)
				must(err)
			}
			if response == nil {
				panic(fmt.Sprintf("provider fixture %s returned no response", name))
			}
			raw, err := json.Marshal(response.Raw)
			must(err)
			snapshot, err := json.Marshal(agent.BuildLLMResponseSnapshot(response))
			must(err)
			fixture.RawJSON = string(raw)
			fixture.SnapshotJSON = string(snapshot)
			if fixture.Streaming && fixture.Method == "complete" {
				stream, err := model.StreamResponse(ctx, request)
				must(err)
				for event := range stream.Events {
					must(event.Error)
				}
				response = stream.Final()
				if response == nil {
					panic(fmt.Sprintf("provider fixture %s returned no stream response", name))
				}
				raw, err := json.Marshal(response.Raw)
				must(err)
				snapshot, err := json.Marshal(agent.BuildLLMResponseSnapshot(response))
				must(err)
				fixture.StreamRawJSON = string(raw)
				fixture.StreamSnapshotJSON = string(snapshot)
			}
			cases[name] = fixture
		}()
	}
	return cases
}
