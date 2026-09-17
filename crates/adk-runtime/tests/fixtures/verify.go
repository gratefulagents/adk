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

func main() {
	data, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	var checkpoints []sdk.DurableCheckpoint
	if err := json.Unmarshal(data, &checkpoints); err != nil {
		panic(err)
	}
	for _, cp := range checkpoints {
		agent := &sdk.Agent{Name: cp.AgentName}
		result, err := sdk.NewRunnerWithModel(nil).Run(context.Background(), agent, nil, sdk.RunConfig{Durable: &sdk.DurableRunConfig{Resume: &cp}})
		if cp.Boundary == sdk.DurableBoundaryRunCompleted {
			if err != nil {
				panic(err)
			}
			if result.FinalText() != "answer" {
				panic("terminal output changed")
			}
		} else if err == nil || !strings.Contains(err.Error(), "requires effect reconciliation") {
			panic(fmt.Sprintf("unsafe checkpoint not gated: %v", err))
		}
	}
	fmt.Printf("Go reader verified %d Rust runner checkpoints, terminal output and nonterminal replay gates\n", len(checkpoints))
}
