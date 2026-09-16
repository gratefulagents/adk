// Platform test-vector derivative: AGPL-3.0; see fixtures/NOTICE.md and licenses/.
// Injected as a virtual test file by go -overlay; never copied into the repository.
package main

import (
	"encoding/json"
	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
	"os"
	"testing"
)

func TestExportMigrationReference(t *testing.T) {
	items := sampleTranscriptItems()
	persisted, ok := persistedItemsFromRun(items)
	if !ok {
		t.Fatal("persist items")
	}
	snap := transcriptSnapshot{Version: transcriptSnapshotVersion, FloorMessageID: 3, SeenMessageID: 17, SelfAssistantMessageID: 18, Items: persisted}
	encoded, err := encodeTranscriptSnapshot(snap)
	if err != nil {
		t.Fatal(err)
	}
	restored, err := decodeTranscriptSnapshot(encoded)
	if err != nil {
		t.Fatal(err)
	}
	payload := map[string]any{
		"schema_version":    1,
		"platform_revision": "08e65c970830f05042c251bcbb46ec6a9e3719b9",
		"cases": []any{map[string]any{
			"name":      "platform-all-item-types-roundtrip",
			"operation": "persist_transcript",
			"input":     map[string]any{"version": transcriptSnapshotVersion, "floor_message_id": 3, "seen_message_id": 17, "self_assistant_message_id": 18, "items": agent.SnapshotRunItems(items)},
			"expected":  restored,
		}},
	}
	data, err := json.MarshalIndent(payload, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("MIGRATION_FIXTURE_OUT"), append(data, '\n'), 0600); err != nil {
		t.Fatal(err)
	}
}
