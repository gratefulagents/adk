// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"testing"
)

func TestNativeActionAuthorizationReference(t *testing.T) {
	project := func(a ActionAuthorization) map[string]any {
		return map[string]any{"decision": a.Decision, "reason": a.Reason, "rule": a.Rule, "risk": a.Risk}
	}
	normalized := []map[string]any{}
	for _, decision := range []string{"allow", "deny", "ask", "classify", "", "unknown", "ALLOW", " allow "} {
		for _, risk := range []string{"", "low", "medium", "high", "unknown"} {
			for _, failed := range []bool{false, true} {
				input := ActionAuthorization{Decision: ActionDecision(decision), Risk: ActionRisk(risk), Reason: "detail <&>", Rule: "custom-rule"}
				var err error
				if failed {
					err = errors.New("private classifier failure")
				}
				normalized = append(normalized, map[string]any{"input": project(input), "failed": failed, "output": project(NormalizeActionAuthorization(input, err))})
			}
		}
	}
	runs := []map[string]any{}
	cases := []struct {
		Decision                 string
		Fail, Approval, Disabled bool
	}{
		{"allow", false, false, false}, {"deny", false, false, false}, {"ask", false, false, false}, {"allow", false, true, false}, {"allow", true, false, false}, {"classify", false, false, false}, {"", false, false, false}, {"ALLOW", false, false, false}, {"allow", false, false, true},
	}
	for _, c := range cases {
		for _, input := range []string{`{"target":"fixture"}`, "", "null", "{}", `{"x":1,"x":2}`, `{"x":"\ud800"}`, "not-json", `"猫💥"`} {
			executed := false
			cfg := RunConfig{WorkDir: "/workspace/repo"}
			if !c.Disabled {
				cfg.ActionAuthorizer = actionAuthorizerFunc(func(_ context.Context, request ActionRequest) (ActionAuthorization, error) {
					a := ActionAuthorization{Decision: ActionDecision(c.Decision), Reason: " boundary ", Rule: "custom-rule", Risk: "retained-risk"}
					if c.Fail {
						return a, errors.New("private classifier failure")
					}
					return a, nil
				})
			}
			response := authorizationToolCallResponse("call-1", "write_file")
			response.Items[0].ToolCall.Input = json.RawMessage(input)
			result, err := NewRunnerWithModel(&mockModel{responses: []*ModelResponse{response}}).Run(context.Background(), &Agent{Name: "test", Tools: []Tool{actionTestTool("write_file", &executed, c.Approval)}, ToolUseBehavior: StopOnFirstTool}, nil, cfg)
			if err != nil {
				t.Fatal(err)
			}
			audits := []map[string]any{}
			for _, a := range result.ActionAuditRecords {
				audits = append(audits, map[string]any{"request": map[string]any{"tool_name": a.Request.ToolName, "input": string(a.Request.Input), "work_dir": a.Request.WorkDir, "read_only": a.Request.ReadOnly, "provenance": a.Request.Provenance}, "authorization": project(a.Authorization), "error": a.Error})
			}
			var output any
			for _, item := range result.NewItems {
				if item.ToolOutput != nil {
					output = map[string]any{"text": item.ToolOutput.Content, "error": item.ToolOutput.IsError}
					break
				}
			}
			runs = append(runs, map[string]any{"input": input, "decision": c.Decision, "failed": c.Fail, "approval": c.Approval, "disabled": c.Disabled, "executed": executed, "interrupted": result.IsInterrupted(), "audits": audits, "output": output})
		}
	}
	data, err := json.Marshal(map[string]any{"normalizations": normalized, "runs": runs})
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("ACTION_AUTHORIZATION_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
