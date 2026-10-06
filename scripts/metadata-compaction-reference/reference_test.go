// SPDX-License-Identifier: GPL-3.0-only
package openai

import (
	"context"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"sync/atomic"
	"testing"
	"time"
	"unicode"
	"unicode/utf8"
)

func TestNativeMetadataReference(t *testing.T) {
	const catalog = `{"models":[{"slug":"GPT-CUSTOM","context_window":10000},{"slug":"vendor/gpt-custom","context_window":20000},{"slug":"Δ","context_window":4000},{"slug":"İ","context_window":6000},{"slug":"no-context"}]}`
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if r.Header.Get("Authorization") != "Bearer fixture-token" {
			t.Error("missing explicit auth")
		}
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(catalog))
	}))
	defer server.Close()
	resolver := NewCompactionMetadataResolver(server.URL, NewAPIKeyAuthSession("fixture-token"))
	if requests.Load() != 0 {
		t.Fatal("construction performed I/O")
	}
	var cases []map[string]any
	for _, model := range []string{" GPT-CUSTOM ", "vendor/gpt-custom", "other/GPT-CUSTOM", "other/Δ", "i", "no-context", "missing", "missing", "nested/other/gpt-custom"} {
		meta, found := resolver.Lookup(context.Background(), model)
		trigger, target, valid := CompactionDefaultsFromModelMetadata(meta)
		cases = append(cases, map[string]any{"model": model, "id": meta.ID, "found": found, "valid": valid, "trigger": trigger, "target": target, "requests": requests.Load()})
	}
	var failed atomic.Int32
	failure := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if failed.Add(1) == 1 {
			w.WriteHeader(500)
			return
		}
		w.Write([]byte(catalog))
	}))
	defer failure.Close()
	retry := NewCompactionMetadataResolver(failure.URL, NewAPIKeyAuthSession("fixture-token"))
	_, first := retry.Lookup(context.Background(), "gpt-custom")
	_, second := retry.Lookup(context.Background(), "gpt-custom")
	before := failed.Load()
	retry.lastAttempt = time.Now().Add(-31 * time.Second)
	_, third := retry.Lookup(context.Background(), "gpt-custom")
	hash := sha256.New()
	count := 0
	for r := rune(0); r <= unicode.MaxRune; r++ {
		if !utf8.ValidRune(r) {
			continue
		}
		keys := modelMetadataLookupKeys(string(r))
		key := ""
		if len(keys) > 0 {
			key = keys[0]
		}
		var size [4]byte
		binary.LittleEndian.PutUint32(size[:], uint32(len(key)))
		hash.Write(size[:])
		hash.Write([]byte(key))
		count++
	}
	out := map[string]any{"unicode_version": unicode.Version, "scalar_count": count, "scalar_sha256": hex.EncodeToString(hash.Sum(nil)), "catalog": catalog, "cases": cases, "retry": map[string]any{"found": []bool{first, second, third}, "requests_before_cooldown": before, "requests_after_cooldown": failed.Load()}}
	data, err := json.Marshal(out)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
