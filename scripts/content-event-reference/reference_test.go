// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/json"
	"os"
	"strings"
	"testing"
)

func TestNativeContentEventReference(t *testing.T) {
	cases := []ContentEvent{
		{},
		{Type: "unknown"},
		{Type: "tool_end", AgentName: "researcher", Tool: "Bash", Status: "failed", IsError: true, FailureKind: "execution", Reason: " reason\nsecond", Message: " message\nsecond", Output: "exit 2\nbad"},
		{Type: "tool_end", Output: "successful output is omitted"},
		{Type: "tool_end", IsError: true},
		{Type: "event", Reason: " \t ", Message: "\n", IsError: true, Output: " "},
		{Type: "event", Reason: " first \r\nsecond ", Message: " third \nlast "},
		{Type: "event", AgentName: " raw agent ", Tool: "raw\nname", Status: " raw status ", FailureKind: "raw\nfailure"},
		{Type: "event", Reason: "\u0085界😀\u00a0", Message: "\ufeffpreserved"},
		{Type: "event", Reason: strings.Repeat("long", 1000), IsError: true, Output: strings.Repeat("output", 1000)},
	}
	observations := []map[string]any{}
	for _, event := range cases {
		observations = append(observations, map[string]any{"event": event, "compact": CompactContentEvent(event)})
	}
	data, err := json.MarshalIndent(map[string]any{"cases": observations}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
