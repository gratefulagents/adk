// Run from repos/sdk: go run ../../fixtures/durable/generate.go generate ../../fixtures/durable
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
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
		panic("usage: generate.go generate|verify|verify-document|verify-records PATH")
	}
	dir := os.Args[2]
	if os.Args[1] == "verify-records" {
		verifyRecords(dir)
		return
	}
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
	writeRecordProof(dir, snap, events[0], old)
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

type recordCase struct {
	Record   string          `json:"record"`
	Case     string          `json:"case"`
	Input    json.RawMessage `json:"input"`
	Expected json.RawMessage `json:"expected"`
}

func typedRecord(name string) any {
	switch name {
	case "RunSnapshot":
		return &d.RunSnapshot{}
	case "Effect":
		return &d.Effect{}
	case "Event":
		return &d.Event{}
	case "ToolCall":
		return &d.ToolCall{}
	case "Step":
		return &d.Step{}
	case "Approval":
		return &d.Approval{}
	case "BudgetCounters":
		return &d.BudgetCounters{}
	case "ChildRun":
		return &d.ChildRun{}
	case "Lease":
		return &d.Lease{}
	case "Attempt":
		return &d.Attempt{}
	case "Cancellation":
		return &d.Cancellation{}
	case "RecoveryDecision":
		return &d.RecoveryDecision{}
	default:
		panic(name)
	}
}
func writeRecordProof(dir string, source d.RunSnapshot, event d.Event, old d.V1Document) {
	// Clone before filling records so the original persistence fixtures stay unchanged.
	b, err := json.Marshal(source)
	must(err)
	var snap d.RunSnapshot
	must(json.Unmarshal(b, &snap))
	now := snap.UpdatedAt
	end := now.Add(2*time.Second + 7*time.Nanosecond)
	snap.Revision = 9007199254740993
	snap.EventSequence = 9007199254740993
	snap.Attempts[0].EndedAt, snap.Attempts[0].Outcome = end, "completed"
	snap.Steps[0].EndedAt = end
	snap.ToolCalls[0].EndedAt = end
	snap.Approvals[0].ResolvedAt, snap.Approvals[0].ResolvedBy = end, "reviewer"
	snap.ChildRuns[0].EndedAt = end
	snap.Cancellation.AcknowledgedAt = end
	event.Sequence = 9007199254740993
	event.Payload = json.RawMessage(`{"n":9007199254740993,"at":"2025-01-02T03:04:05.123456789+02:00"}`)
	snap.Steps[0].Status = "2025-01-02T03:04:05.123456789+02:00"
	snap.Steps[0].Data = json.RawMessage(`{"ended_at":"2025-01-02T03:04:05.123456789+02:00"}`)
	records := []any{snap, snap.Effects[0], event, snap.ToolCalls[0], snap.Steps[0], snap.Approvals[0], snap.CumulativeBudget, snap.ChildRuns[0], d.Lease{TenantID: "tenant_go", RunID: "run_go", Owner: "worker", Token: "lease_go", ExpiresAt: end}, snap.Attempts[0], *snap.Cancellation, d.RecoveryDecision{Action: d.RecoveryRetry, Automatic: true, IdempotencyKey: "record-key"}}
	var cases []recordCase
	add := func(name, kind string, input []byte) {
		value := typedRecord(name)
		must(json.Unmarshal(input, value))
		expected, err := json.Marshal(value)
		must(err)
		cases = append(cases, recordCase{name, kind, input, expected})
	}
	for _, record := range records {
		t := reflect.TypeOf(record)
		name := t.Name()
		b, err := json.Marshal(record)
		must(err)
		add(name, "populated", b)
		add(name, "omitted", []byte(`{}`))
		nulls := map[string]any{}
		for i := 0; i < t.NumField(); i++ {
			nulls[strings.Split(t.Field(i).Tag.Get("json"), ",")[0]] = nil
		}
		b, err = json.Marshal(nulls)
		must(err)
		add(name, "null", b)
		unknown := reflect.New(t).Elem()
		unknown.Set(reflect.ValueOf(record))
		changed := false
		for i := 0; i < t.NumField(); i++ {
			switch t.Field(i).Type.Name() {
			case "RunStatus", "DataClassification", "EffectClassification", "EffectState", "RecoveryAction":
				unknown.Field(i).SetString("Future / 世界 <v3>\n" + t.Field(i).Type.Name())
				changed = true
			}
		}
		if changed {
			b, err = json.Marshal(unknown.Interface())
			must(err)
			add(name, "unknown", b)
		}
	}
	stamps := []string{
		"2025-01-02T03:04:05Z",
		"2025-01-02T03:04:05.1+05:45",
		"2025-01-02T03:04:05.12-03:30",
		"2025-01-02T03:04:05.123Z",
		"2025-01-02T03:04:05.1234+05:45",
		"2025-01-02T03:04:05.12345-03:30",
		"2025-01-02T03:04:05.123456Z",
		"2025-01-02T03:04:05.1234567+05:45",
		"2025-01-02T03:04:05.12345678-03:30",
		"2025-01-02T03:04:05.123456789Z",
		"2025-01-02T03:04:05.000000001-03:30",
		"2025-01-02T03:04:05.100000000+00:00",
		"2025-01-02T03:04:05.000000000-00:00",
		"0001-01-01T00:00:00Z",
	}
	var replaceTimes func(map[string]any, string) bool
	replaceTimes = func(fields map[string]any, stamp string) bool {
		changed := false
		for key, child := range fields {
			switch key {
			case "started_at", "ended_at", "requested_at", "resolved_at", "acknowledged_at", "prepared_at", "updated_at", "created_at", "retain_until", "expires_at", "at":
				fields[key] = stamp
				changed = true
			case "attempts", "steps", "tool_calls", "approvals", "child_runs", "effects":
				if children, ok := child.([]any); ok {
					for _, item := range children {
						replaceTimes(item.(map[string]any), stamp)
					}
				}
			case "cancellation":
				replaceTimes(child.(map[string]any), stamp)
			}
		}
		return changed
	}
	for i, stamp := range stamps {
		for _, record := range records {
			b, err := json.Marshal(record)
			must(err)
			fields := exactJSON(b).(map[string]any)
			if !replaceTimes(fields, stamp) {
				continue
			}
			b, err = json.Marshal(fields)
			must(err)
			add(reflect.TypeOf(record).Name(), fmt.Sprintf("timestamp_%02d", i), b)
		}
	}
	write(dir, "records.json", cases)
	old.Status = d.RunStatus("Future / 世界 <v3>\nRunStatus")
	old.Events[0].Classification = d.DataClassification("Future / 世界 <v3>\nDataClassification")
	write(dir, "v1-unknown.json", old)
	b, err = json.Marshal(old)
	must(err)
	migrated, err := d.DecodeDocument(b)
	must(err)
	write(dir, "v1-unknown-migrated.json", migrated)
}
func exactJSON(b []byte) any {
	dec := json.NewDecoder(bytes.NewReader(b))
	dec.UseNumber()
	var value any
	must(dec.Decode(&value))
	return value
}
func verifyRecords(dir string) {
	var cases []recordCase
	must(json.Unmarshal(read(filepath.Join(dir, "records.json")), &cases))
	var rust []json.RawMessage
	must(json.Unmarshal(read(filepath.Join(dir, "rust-records.json")), &rust))
	if len(cases) != len(rust) {
		panic("record count")
	}
	for i, c := range cases {
		value := typedRecord(c.Record)
		must(json.Unmarshal(rust[i], value))
		actual, err := json.Marshal(value)
		must(err)
		if !reflect.DeepEqual(exactJSON(rust[i]), exactJSON(c.Expected)) || !reflect.DeepEqual(exactJSON(actual), exactJSON(c.Expected)) {
			panic(c.Record + "/" + c.Case)
		}
	}
	fmt.Printf("Go decoded and compared %d Rust-encoded typed record cases against independent Go expectations\n", len(cases))
}
