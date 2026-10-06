// SPDX-License-Identifier: GPL-3.0-only
package openai

import (
 "context"
 "encoding/json"
 "net/http"
 "net/http/httptest"
 "os"
 "sync/atomic"
 "testing"
 "time"
)

func TestNativeMetadataReference(t *testing.T) {
 const catalog = `{"models":[{"slug":"GPT-CUSTOM","context_window":10000},{"slug":"vendor/gpt-custom","context_window":20000},{"slug":"Δ","context_window":4000},{"slug":"İ","context_window":6000}]}`
 var requests atomic.Int32
 server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
  requests.Add(1)
  if r.Header.Get("Authorization") != "Bearer fixture-token" { t.Error("missing explicit auth") }
  w.Header().Set("Content-Type", "application/json")
  w.Write([]byte(catalog))
 }))
 defer server.Close()
 resolver := NewCompactionMetadataResolver(server.URL, NewAPIKeyAuthSession("fixture-token"))
 if requests.Load() != 0 { t.Fatal("construction performed I/O") }
 var cases []map[string]any
 for _, model := range []string{" GPT-CUSTOM ", "vendor/gpt-custom", "other/GPT-CUSTOM", "other/Δ", "i", "missing", "missing", "nested/other/gpt-custom"} {
  meta, found := resolver.Lookup(context.Background(), model)
  trigger, target, valid := CompactionDefaultsFromModelMetadata(meta)
  cases = append(cases, map[string]any{"model":model,"found":found,"valid":valid,"trigger":trigger,"target":target,"requests":requests.Load()})
 }
 var failed atomic.Int32
 failure := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
  if failed.Add(1) == 1 { w.WriteHeader(500); return }
  w.Write([]byte(catalog))
 }))
 defer failure.Close()
 retry := NewCompactionMetadataResolver(failure.URL, NewAPIKeyAuthSession("fixture-token"))
 _, first := retry.Lookup(context.Background(), "gpt-custom")
 _, second := retry.Lookup(context.Background(), "gpt-custom")
 before := failed.Load()
 retry.lastAttempt = time.Now().Add(-31*time.Second)
 _, third := retry.Lookup(context.Background(), "gpt-custom")
 out := map[string]any{"catalog":catalog,"cases":cases,"retry":map[string]any{"found":[]bool{first,second,third},"requests_before_cooldown":before,"requests_after_cooldown":failed.Load()}}
 data, err := json.Marshal(out); if err != nil { t.Fatal(err) }
 if err := os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil { t.Fatal(err) }
}
