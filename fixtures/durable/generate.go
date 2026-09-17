// Run from repos/sdk: go run ../../fixtures/durable/generate.go generate ../../fixtures/durable
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"

	d "github.com/gratefulagents/sdk/pkg/agentsdk/durable"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func write(dir, name string, value any) {
	b, err := json.MarshalIndent(value, "", "  ")
	must(err)
	must(os.WriteFile(filepath.Join(dir, name), append(b, '\n'), 0600))
}
func read(path string) []byte { b, err := os.ReadFile(path); must(err); return b }

type xor struct{}

func (xor) Encrypt(_ context.Context, b []byte) ([]byte, error) {
	out := append([]byte(nil), b...)
	for i := range out {
		out[i] ^= 0x5a
	}
	return out, nil
}
func (x xor) Decrypt(c context.Context, b []byte) ([]byte, error) { return x.Encrypt(c, b) }
func main() {
	if len(os.Args) != 3 {
		panic("usage: generate.go generate|verify|verify-document PATH")
	}
	dir := os.Args[2]
	ctx := context.Background()
	if os.Args[1] == "verify-document" {
		doc, err := d.DecodeDocument(read(dir))
		must(err)
		if doc.SchemaVersion != 2 || doc.Snapshot.Revision != 1 || len(doc.Events) != 1 || doc.Events[0].Sequence != 1 {
			panic("unexpected Rust document")
		}
		if doc.Snapshot.Effects[0].IdempotencyKey != d.IdempotencyKey(doc.Snapshot.RunID, doc.Snapshot.Effects[0].ID) {
			panic("unstable Rust key")
		}
		fmt.Println("Go decoded Rust document with typed boundaries, effect key and event sequence")
		return
	}
	if os.Args[1] == "verify" {
		for _, encrypted := range []bool{false, true} {
			name := "plain"
			opts := d.FilesystemOptions{}
			if encrypted {
				name = "encrypted"
				opts.Encryptor = xor{}
			}
			store, err := d.NewFilesystemStore(filepath.Join(dir, name), opts)
			must(err)
			snap, events, err := store.Load(ctx, "tenant_go", "run_go")
			must(err)
			if snap.Revision != 1 || len(events) != 1 || events[0].Sequence != 1 || snap.State == nil || len(snap.Effects) != 1 {
				panic("Rust store state lost")
			}
			lease, err := store.AcquireLease(ctx, "tenant_go", "run_go", "go-resumer", time.Minute)
			must(err)
			snap.Revision++
			_, err = store.Append(ctx, lease, 1, []d.Event{{Type: "go.resumed"}}, snap)
			must(err)
			must(store.ReleaseLease(ctx, lease))
		}
		fmt.Println("Go loaded and resumed Rust plaintext and encrypted filesystem stores")
		return
	}
	if os.Args[1] != "generate" {
		panic("unknown command")
	}
	must(os.MkdirAll(dir, 0700))
	now := time.Date(2026, 1, 2, 3, 4, 5, 123456789, time.FixedZone("fixture", 3600))
	old := d.V1Document{SchemaVersion: 1, TenantID: "tenant_go", RunID: "run_go", Revision: 4, Cancelled: true, BudgetTokens: 123, CreatedAt: now, UpdatedAt: now, Events: []d.Event{{ID: "event_legacy", Type: "legacy", Payload: json.RawMessage(`{"n":9007199254740993}`)}}}
	write(dir, "v1.json", old)
	b, err := json.Marshal(old)
	must(err)
	migrated, err := d.DecodeDocument(b)
	must(err)
	write(dir, "v1-migrated.json", migrated)
	snap := d.NewRunSnapshot("tenant_go", "run_go", now)
	snap.Revision = 1
	snap.EventSequence = 1
	snap.Status = d.RunRunning
	snap.Classification = d.DataSensitive
	snap.State = json.RawMessage(`{"checkpoint":"tool_prepared","large":18446744073709551615,"text":"<hello>&世界"}`)
	snap.Attempts = []d.Attempt{{ID: "att_go", StartedAt: now}}
	snap.Steps = []d.Step{{ID: "step_go", Kind: "tool", Status: "prepared", Data: json.RawMessage(`{"nested":true}`), StartedAt: now}}
	snap.ToolCalls = []d.ToolCall{{ID: "tool_go", Name: "write", Status: "prepared", Classification: d.DataSecret, Input: json.RawMessage(`{"key":"secret"}`), Output: json.RawMessage(`{"ok":true}`), StartedAt: now}}
	snap.Approvals = []d.Approval{{ID: "approval_go", Status: "pending", RequestedAt: now}}
	snap.ChildRuns = []d.ChildRun{{ID: "child_go", RunID: "run_child", Status: d.RunRunning, StartedAt: now}}
	snap.Effects = []d.Effect{{ID: "effect_go", Classification: d.EffectNonReplayable, DataClassification: d.DataSecret, State: d.EffectOutcomeUnknown, IdempotencyKey: d.IdempotencyKey("run_go", "effect_go"), PreparedAt: now, UpdatedAt: now, Outcome: json.RawMessage(`{"uncertain":true}`)}}
	snap.Cancellation = &d.Cancellation{RequestedAt: now, RequestedBy: "operator", Reason: "stop"}
	snap.CumulativeBudget = d.BudgetCounters{InputTokens: 9007199254740993, OutputTokens: 12, ToolCalls: 2, CostMicros: 42, WallTimeMS: 99}
	deadline := now.Add(time.Hour)
	snap.RetainUntil = &deadline
	events := []d.Event{{ID: "event_go", TenantID: "tenant_go", RunID: "run_go", Sequence: 1, At: now, Type: "tool.prepared", Classification: d.DataSensitive, Payload: json.RawMessage(`{"n":9007199254740993}`)}}
	doc := d.Document{SchemaVersion: 2, Snapshot: snap, Events: events}
	b, err = d.EncodeDocument(doc)
	must(err)
	// Write bytes directly: decoding through interface{} would round large numbers.
	must(os.WriteFile(filepath.Join(dir, "v2.json"), b, 0600))
	write(dir, "snapshot.json", snap)
	write(dir, "event.json", events[0])
	record := struct {
		Document d.Document `json:"document"`
		Lease    *d.Lease   `json:"lease,omitempty"`
	}{doc, &d.Lease{TenantID: "tenant_go", RunID: "run_go", Owner: "expired-go", Token: "lease_go", ExpiresAt: now}}
	write(dir, "filesystem.json", record)
	plain, err := json.Marshal(record)
	must(err)
	cipher, err := (xor{}).Encrypt(ctx, plain)
	must(err)
	write(dir, "filesystem-encrypted.json", struct {
		Encrypted bool   `json:"encrypted"`
		Data      []byte `json:"data"`
	}{true, cipher})
	var recovery []any
	for _, class := range []d.EffectClassification{d.EffectIdempotent, d.EffectDeduplicated, d.EffectNonReplayable} {
		for _, state := range []d.EffectState{d.EffectPrepared, d.EffectDispatched, d.EffectSucceeded, d.EffectFailed, d.EffectOutcomeUnknown} {
			effect := d.Effect{ID: "effect_go", Classification: class, State: state, IdempotencyKey: d.IdempotencyKey("run_go", "effect_go"), PreparedAt: now, UpdatedAt: now}
			recovery = append(recovery, struct {
				Effect   d.Effect           `json:"effect"`
				Decision d.RecoveryDecision `json:"decision"`
			}{effect, d.RecoverEffect(effect)})
		}
	}
	write(dir, "recovery.json", recovery)
	fmt.Println("Generated durable fixtures with baseline Go durable package")
}
