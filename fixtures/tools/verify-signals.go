// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../fixtures/tools/verify-signals.go
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tools/signal"
)

func main() {
	data, err := os.ReadFile("../../fixtures/tools/signals.json")
	if err != nil {
		panic(err)
	}
	var fixtures []struct {
		Name        string          `json:"name"`
		Input       json.RawMessage `json:"input"`
		Text        string          `json:"text"`
		IsError     bool            `json:"is_error"`
		ShouldPause bool            `json:"should_pause"`
	}
	if err := json.Unmarshal(data, &fixtures); err != nil {
		panic(err)
	}
	tools := map[string]agentsdk.Tool{
		"think":           &signal.ThinkTool{},
		"AskUserQuestion": &signal.AskUserQuestionTool{},
		"present_plan":    &signal.PresentPlanTool{},
		"finish":          &signal.FinishTool{},
	}
	for i, fixture := range fixtures {
		result, err := tools[fixture.Name].Execute(context.Background(), fixture.Input, "")
		if err != nil || result.Content != fixture.Text || result.IsError != fixture.IsError || result.ShouldPause != fixture.ShouldPause {
			panic(fmt.Sprintf("fixture %d %s: result=%+v err=%v", i, fixture.Name, result, err))
		}
	}
	fmt.Printf("%d SDK signal fixtures verified\n", len(fixtures))
}
