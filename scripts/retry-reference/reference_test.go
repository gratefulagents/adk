// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"encoding/json"
	"os"
	"strconv"
	"testing"
)

func TestNativeRetryReference(t *testing.T) {
	rows := []map[string]any{}
	for _, initial := range []int{-1, 0, 1, 1000, 30001, 9223372036854775807} {
		for _, maximum := range []int{-1, 0, 1, 30000, 9223372036854775807} {
			for _, text := range []string{"0", "0.5", "1", "1.5", "2", "10", "NaN", "+Inf", "-Inf"} {
				multiplier, err := strconv.ParseFloat(text, 64)
				if err != nil {
					t.Fatal(err)
				}
				for _, attempt := range []int{-1, 0, 1, 2, 10, 63, 9223372036854775807} {
					p := RetryPolicy{MaxRetries: -3, Backoff: RetryBackoffSettings{InitialDelayMS: initial, MaxDelayMS: maximum, Multiplier: multiplier}}
					rows = append(rows, map[string]any{"initial": initial, "maximum": maximum, "multiplier": text, "attempt": attempt, "delay": int64(p.DelayForAttempt(attempt))})
				}
			}
		}
	}
	p := DefaultRetryPolicy()
	output := map[string]any{"cases": rows, "default": map[string]any{"max_retries": p.MaxRetries, "initial": p.Backoff.InitialDelayMS, "maximum": p.Backoff.MaxDelayMS, "multiplier": p.Backoff.Multiplier, "attempt_2": int64(p.DelayForAttempt(2))}}
	data, err := json.Marshal(output)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("RETRY_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
