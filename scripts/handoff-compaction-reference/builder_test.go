// SPDX-License-Identifier: GPL-3.0-only
package runtime

import (
	"encoding/json"
	"github.com/gratefulagents/sdk/pkg/agentsdk"
	"os"
	"testing"
)

func TestNativeHandoffCompactionReference(t *testing.T) {
	cases := []map[string]any{}
	for _, legacy := range []bool{false, true} {
		for _, mode := range []string{"legacy", "off", "on", "explicit-off", "explicit-on"} {
			cfg := Config{EnableCompaction: legacy}
			if mode != "legacy" {
				cfg.Features = &Features{Runtime: RuntimeFeatures{HandoffHistory: mode == "on"}}
			}
			if mode == "explicit-off" || mode == "explicit-on" {
				cfg.HandoffHistory = &agentsdk.HandoffHistoryConfig{Enabled: mode == "explicit-on", MaxTokens: 300, TargetTokens: 120, PreserveRecentItems: 3, SummaryBulletLimit: 2}
			}
			cases = append(cases, map[string]any{"legacy": legacy, "mode": mode, "policy": runHandoffHistory(cfg)})
		}
	}
	data, err := json.MarshalIndent(cases, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT")+".builder", data, 0600); err != nil {
		t.Fatal(err)
	}
}
