package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"sync/atomic"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	sdkruntime "github.com/gratefulagents/sdk/pkg/agentsdk/runtime"
)

type subagentSelection struct {
	Task    bool `json:"task"`
	Status  bool `json:"status"`
	Control bool `json:"control"`
}
type subagentSelectionCase struct {
	Selection subagentSelection `json:"selection"`
	Tools     []string          `json:"tools"`
	Scheduler bool              `json:"scheduler"`
}

func generateSubagents() ([]byte, error) {
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		calls.Add(1)
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer server.Close()
	root, err := os.MkdirTemp("", "adk-subagent-selection-")
	if err != nil {
		return nil, err
	}
	defer os.RemoveAll(root)
	result := struct {
		SchemaVersion int                     `json:"schema_version"`
		SDKRevision   string                  `json:"sdk_revision"`
		Cases         []subagentSelectionCase `json:"cases"`
	}{SchemaVersion: 1, SDKRevision: sdkRevision}
	for mask := 0; mask < 8; mask++ {
		selection := subagentSelection{mask&1 != 0, mask&2 != 0, mask&4 != 0}
		cfg := sdkruntime.Config{
			Provider: "openai", Model: "gpt-4.1", BaseURL: server.URL + "/v1",
			APIKey: "offline-fixture-not-a-credential", WorkDir: root,
			RoleCatalog: agentsdk.RoleCatalog{{Name: "worker", Instructions: "Never execute a model in this composition fixture."}},
			Features: &sdkruntime.Features{SubAgents: sdkruntime.SubAgentFeatures{
				Async: sdkruntime.AsyncSubAgentFeatures{Task: selection.Task, Status: selection.Status, Control: selection.Control},
			}},
		}
		bundle, err := sdkruntime.NewBuilder(cfg).Build(context.Background())
		if err != nil {
			return nil, fmt.Errorf("mask %d: %w", mask, err)
		}
		observation := subagentSelectionCase{Selection: selection, Tools: []string{}, Scheduler: bundle.SessionState.SubAgentScheduler() != nil}
		for _, tool := range bundle.Tools {
			observation.Tools = append(observation.Tools, tool.Name())
		}
		for i := len(bundle.Closers) - 1; i >= 0; i-- {
			if err := bundle.Closers[i].Close(); err != nil {
				return nil, err
			}
		}
		result.Cases = append(result.Cases, observation)
	}
	if calls.Load() != 0 {
		return nil, fmt.Errorf("composition unexpectedly made %d provider requests", calls.Load())
	}
	b, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(b, '\n'), nil
}
