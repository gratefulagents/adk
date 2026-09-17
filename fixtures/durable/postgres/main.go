package main

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"time"

	d "github.com/gratefulagents/sdk/pkg/agentsdk/durable"
	_ "github.com/lib/pq"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}

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
		panic("usage: postgres-interop seed|verify SCHEMA")
	}
	schema := os.Args[2]
	if !strings.HasPrefix(schema, "durable_test_") || strings.ContainsAny(schema, " ;'\"") {
		panic("not a test schema")
	}
	db, err := sql.Open("postgres", os.Getenv("ADK_TEST_POSTGRES_URL"))
	must(err)
	defer db.Close()
	db.SetMaxOpenConns(1)
	_, err = db.Exec("SET search_path TO " + schema)
	must(err)
	ctx := context.Background()
	for _, encrypted := range []bool{false, true} {
		tenant := d.TenantID("go_pg_plain")
		options := d.PostgresOptions{}
		if encrypted {
			tenant = "go_pg_encrypted"
			options.Encryptor = xor{}
		}
		store, err := d.NewPostgresStore(db, options)
		must(err)
		must(store.Init(ctx))
		if os.Args[1] == "seed" {
			snap := d.NewRunSnapshot(tenant, "run_go", time.Now())
			snap.State = json.RawMessage(`{"checkpoint":"prepared","large":9007199254740993}`)
			effect := d.NewEffect(snap.RunID, d.EffectNonReplayable, time.Now())
			snap.Effects = []d.Effect{effect}
			snap.CumulativeBudget.InputTokens = 100
			must(store.Create(ctx, snap))
			lease, err := store.AcquireLease(ctx, tenant, snap.RunID, "go", time.Minute)
			must(err)
			snap.Revision = 1
			must(d.TransitionEffect(&snap.Effects[0], d.EffectDispatched, time.Now()))
			_, err = store.Append(ctx, lease, 0, []d.Event{{Type: "go.dispatched", Payload: json.RawMessage(`{"large":9007199254740993}`)}}, snap)
			must(err)
			must(store.ReleaseLease(ctx, lease))
		} else if os.Args[1] == "verify" {
			snap, events, err := store.Load(ctx, tenant, "run_go")
			must(err)
			if snap.Revision != 2 || len(events) != 2 || events[1].Type != "rust.recovered" || snap.CumulativeBudget.InputTokens != 107 {
				panic("Rust continuation mismatch")
			}
			if snap.Effects[0].IdempotencyKey != d.IdempotencyKey(snap.RunID, snap.Effects[0].ID) {
				panic("Rust changed stable effect key")
			}
			decision := d.RecoverEffect(snap.Effects[0])
			if decision.Action != d.RecoveryOperator || decision.Automatic {
				panic("unsafe Rust effect recovery")
			}
			lease, err := store.AcquireLease(ctx, tenant, snap.RunID, "go-resumer", time.Minute)
			must(err)
			snap.Revision = 3
			_, err = store.Append(ctx, lease, 2, []d.Event{{Type: "go.verified"}}, snap)
			must(err)
			must(store.ReleaseLease(ctx, lease))
		} else {
			panic("unknown mode")
		}
	}
	fmt.Println("Go PostgreSQL " + os.Args[1] + " succeeded for plaintext and encrypted bodies")
}
