// SPDX-License-Identifier: GPL-3.0-only
package main

import (
	"encoding/json"
	"math"
	"os"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
)

type observation struct {
	Left  agentsdk.Usage `json:"left"`
	Right agentsdk.Usage `json:"right"`
	Sum   agentsdk.Usage `json:"sum"`
	Total int64          `json:"total"`
}

func main() {
	inputs := []agentsdk.Usage{
		{},
		{Requests: 2, InputTokens: 3, OutputTokens: 5, CacheReadTokens: 7, CacheCreateTokens: 11},
		{Requests: -2, InputTokens: -3, OutputTokens: -5, CacheReadTokens: -7, CacheCreateTokens: -11},
		{Requests: math.MaxInt64, InputTokens: math.MaxInt64, OutputTokens: math.MaxInt64, CacheReadTokens: math.MaxInt64, CacheCreateTokens: math.MaxInt64},
		{Requests: math.MinInt64, InputTokens: math.MinInt64, OutputTokens: math.MinInt64, CacheReadTokens: math.MinInt64, CacheCreateTokens: math.MinInt64},
		{Requests: 1, InputTokens: 1, OutputTokens: 1, CacheReadTokens: 1, CacheCreateTokens: 1},
		{Requests: -1, InputTokens: -1, OutputTokens: -1, CacheReadTokens: -1, CacheCreateTokens: -1},
		{CacheReadTokens: 41, CacheCreateTokens: 43},
	}
	cases := []observation{}
	for _, left := range inputs {
		for _, right := range inputs {
			sum := left
			sum.Add(right)
			cases = append(cases, observation{left, right, sum, sum.TotalTokens()})
		}
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(map[string]any{
		"sdk_revision": "1dc92b73900fac74dc357a938e4b5eee6392b418",
		"source":       "internal/agent/usage.go",
		"license":      "GPL-3.0-only",
		"target":       "64-bit Go int",
		"cases":        cases,
	}); err != nil {
		panic(err)
	}
}
