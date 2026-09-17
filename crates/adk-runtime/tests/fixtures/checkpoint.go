// Run from repos/sdk: go run ../../crates/adk-runtime/tests/fixtures/checkpoint.go
package main
import (
 "encoding/json"
 "os"
 "time"
 sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)
func main() {
 cp := sdk.DurableCheckpoint{SchemaVersion: sdk.DurableCheckpointSchemaVersion,
 RunID: "run-1", AttemptID: "go-attempt", StepID: "step_go", Sequence: 5,
 Boundary: sdk.DurableBoundaryRunStarted, AgentName: "agent",
 History: sdk.SnapshotRunItems([]sdk.RunItem{{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: "Go-authored input"}}}),
 Usage: sdk.Usage{Requests: 2, InputTokens: 11, OutputTokens: 7},
 CreatedAt: time.Date(2026, 1, 2, 3, 4, 5, 0, time.UTC)}
 if err := json.NewEncoder(os.Stdout).Encode(cp); err != nil { panic(err) }
}
