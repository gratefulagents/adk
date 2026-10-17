// SPDX-License-Identifier: GPL-3.0-only
package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"strings"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
)

type observation struct {
	Left  agentsdk.Usage `json:"left"`
	Right agentsdk.Usage `json:"right"`
	Sum   agentsdk.Usage `json:"sum"`
	Total int64          `json:"total"`
}

func decodeObservations() []map[string]any {
	inputs := []string{}
	for _, field := range []string{"requests", "input_tokens", "output_tokens", "cache_read_tokens", "cache_create_tokens"} {
		for _, value := range []string{"-0", "0", "1", "-1", "9223372036854775807", "-9223372036854775808", "9223372036854775808", "-9223372036854775809", "1.0", "1e0", "null", `"1"`, "true", "[]", "{}"} {
			inputs = append(inputs, fmt.Sprintf(`{%q:%s}`, field, value))
		}
		inputs = append(inputs,
			fmt.Sprintf(`{%q:7,%q:null}`, field, field),
			fmt.Sprintf(`{%q:7,%q:8}`, field, strings.ToUpper(field)),
			fmt.Sprintf(`{%q:7,%q:0}`, field, field),
			fmt.Sprintf(`{%q:"invalid",%q:8}`, field, field),
			fmt.Sprintf(`{%q:9}`, strings.ReplaceAll(strings.ReplaceAll(field, "s", "ſ"), "k", "K")),
		)
	}
	inputs = append(inputs, `{}`, `null`, `[]`, `1`, `"usage"`, `true`, `{"unknown":1e9999}`, `{"requests":1} {}`, "{\"\xff\":1}")
	for _, depth := range []int{127, 128, 9999, 10000} {
		inputs = append(inputs, `{"unknown":`+strings.Repeat("[", depth)+"0"+strings.Repeat("]", depth)+`}`)
	}
	cases := make([]map[string]any, 0, len(inputs))
	for index, input := range inputs {
		var usage agentsdk.Usage
		err := json.Unmarshal([]byte(input), &usage)
		row := map[string]any{"name": fmt.Sprintf("usage-%03d", index), "input_hex": hex.EncodeToString([]byte(input)), "accepted": err == nil}
		if err == nil {
			encoded, err := json.Marshal(usage)
			if err != nil {
				panic(err)
			}
			row["wire_json"] = string(encoded)
		}
		cases = append(cases, row)
	}
	return cases
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
		"decode_cases": decodeObservations(),
	}); err != nil {
		panic(err)
	}
}
