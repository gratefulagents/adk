// SPDX-License-Identifier: GPL-3.0-only
// Actual pinned Go reader: Rust extensions are ignored, unsafe boundaries refused.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	"os"
	"strings"
)

type noExecutionModel struct{}

func (noExecutionModel) GetResponse(context.Context, sdk.ModelRequest) (*sdk.ModelResponse, error) {
	panic("checkpoint replay invoked model")
}
func (noExecutionModel) StreamResponse(context.Context, sdk.ModelRequest) (*sdk.ModelStream, error) {
	panic("checkpoint replay invoked streaming model")
}
func (noExecutionModel) GetRetryAdvice(error) *sdk.ModelRetryAdvice { return nil }
func (noExecutionModel) CalculateCost(sdk.Usage) float64            { return 0 }
func (noExecutionModel) Provider() string                           { return "test" }

func main() {
	data, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	var checkpoints []sdk.DurableCheckpoint
	if err := json.Unmarshal(data, &checkpoints); err != nil {
		panic(err)
	}
	var restored, boundaryGated, historyGated int
	for _, cp := range checkpoints {
		agent := &sdk.Agent{Name: cp.AgentName}
		unknownHistory := false
		for _, item := range cp.History {
			unknownHistory = unknownHistory || item.Type == "unknown"
		}
		result, err := sdk.NewRunnerWithModel(noExecutionModel{}).Run(context.Background(), agent, nil, sdk.RunConfig{Durable: &sdk.DurableRunConfig{Resume: &cp}})
		// SDK 1dc92b7 runner.go restores history before checking even terminal boundaries.
		if unknownHistory {
			if err == nil || err.Error() != `restore durable checkpoint: unknown durable run item type "unknown"` {
				panic(fmt.Sprintf("unknown history not gated: %v", err))
			}
			historyGated++
		} else if cp.Boundary == sdk.DurableBoundaryRunCompleted {
			if err != nil {
				panic(err)
			}
			if result.FinalText() != "answer" {
				panic("terminal output changed")
			}
			restored++
		} else if err == nil || !strings.Contains(err.Error(), "requires effect reconciliation") {
			panic(fmt.Sprintf("unsafe checkpoint not gated: %v", err))
		} else {
			boundaryGated++
		}
	}
	if restored == 0 || boundaryGated == 0 || historyGated == 0 {
		panic("missing terminal, boundary or unknown-history coverage")
	}
	fmt.Printf("Go reader verified %d Rust runner checkpoints: %d terminal outputs, %d boundary gates, %d unknown-history gates; no execution\n", len(checkpoints), restored, boundaryGated, historyGated)
}
