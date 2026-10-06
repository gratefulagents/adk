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

func nativeCatalogObservations() []map[string]any {
	cases := []struct{ name, body string }{
		{"exponent-integer", `{"models":[{"slug":"exponent","context_window":1e0}]}`},
		{"unicode-picker", `{"models":[{"slug":"Β","priority":1},{"slug":"α","priority":1},{"slug":"hidden","visibility":" HİDE "},{"slug":"first","priority":2}]}`},
		{"signed-codex", `{"models":[{"slug":"signed","context_window":-1,"max_context_window":10000,"auto_compact_token_limit":-5,"effective_context_window_percent":-7,"priority":-3}]}`},
		{"signed-data", `{"data":[{"id":"signed","capabilities":{"limits":{"max_context_window_tokens":-1,"max_prompt_tokens":10000,"max_output_tokens":-20}}}]}`},
		{"integer-boundaries", `{"models":[{"slug":"max","context_window":9223372036854775807},{"slug":"overflow-negative","context_window":1152921504606846976},{"slug":"min","context_window":-9223372036854775808}]}`},
		{"integer-overflow", `{"models":[{"slug":"overflow","context_window":9223372036854775808}]}`},
		{"negative-zero", `{"models":[{"slug":"zero","context_window":-0,"priority":-0}]}`},
		{"fractional-integer", `{"models":[{"slug":"fractional","context_window":1.0}]}`},
		{"plain-foreign-fields", `{"data":[{"id":"plain","context_window":10000,"auto_compact_token_limit":5000,"display_name":"foreign","visibility":"hide","priority":7,"default_reasoning_level":"high"}]}`},
		{"codex-foreign-fields", `{"models":[{"slug":"codex","id":42,"max_output_tokens":"ignored","capabilities":42}]}`},
		{"codex-wrong-id", `{"models":[{"id":"wrong"}],"data":[{"id":"fallback"}]}`},
		{"plain-wrong-id", `{"data":[{"slug":"wrong"}]}`},
		{"codex-nulls", `{"models":[null,{"slug":"valid","display_name":null,"description":null,"visibility":null,"default_reasoning_level":null,"supported_reasoning_levels":[null,{}, {"effort":null},{"effort":" İ "}],"upgrade":{"model":null}}]}`},
		{"empty-codex-fallback", `{"models":[],"data":[null,{"id":" plain ","capabilities":null}]}`},
		{"null-limits", `{"data":[{"id":"plain","capabilities":{"limits":null}}]}`},
		{"unicode-labels", `{"models":[{"slug":"x","visibility":" HİDE ","default_reasoning_level":" İ ","supported_reasoning_levels":[{"effort":" İ "}]}]}`},
		{"unicode-first-duplicate", `{"models":[{"slug":"Δ","context_window":4000},{"slug":"δ","context_window":8000}]}`},
		{"inactive-schema-validation", `{"models":[{"slug":"valid"}],"data":[{"id":7}]}`},
	}
	var out []map[string]any
	for _, item := range cases {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write([]byte(item.body)) }))
		models, err := FetchModelMetadata(context.Background(), server.URL, NewAPIKeyAuthSession("fixture-token"))
		server.Close()
		var defaults []map[string]any
		for _, model := range models {
			trigger, target, valid := CompactionDefaultsFromModelMetadata(model)
			defaults = append(defaults, map[string]any{"trigger": trigger, "target": target, "valid": valid})
		}
		picker := []string{}
		for _, model := range PickerModelMetadata(models) {
			picker = append(picker, model.ID)
		}
		out = append(out, map[string]any{"picker": picker, "name": item.name, "body": item.body, "error": err != nil, "models": models, "defaults": defaults})
	}
	return out
}

func nativeThresholdObservations() []map[string]any {
	const maxInt = 1<<63 - 1
	const minInt = -1 << 63
	models := []ModelMetadata{
		{ID: "gpt-5.5", ContextWindow: 272000, MaxContextWindow: 272000},
		{ID: "gpt-5.6-sol", ContextWindow: 372000, MaxContextWindow: 372000},
		{ID: "gpt-test", ContextWindow: 1000, AutoCompactTokenLimit: 950},
	}
	for _, contextWindow := range []int{-1, 0, 1, 2, 3, 10000, 1 << 60, maxInt, minInt} {
		for _, maxWindow := range []int{-1, 0, 10000} {
			for _, autoLimit := range []int{-1, 0, 1, 2, 4500, 99999, maxInt, minInt} {
				models = append(models, ModelMetadata{ID: "grid", ContextWindow: contextWindow, MaxContextWindow: maxWindow, AutoCompactTokenLimit: autoLimit, EffectiveContextWindowPercent: 95})
			}
		}
	}
	var out []map[string]any
	for _, model := range models {
		trigger, target, valid := CompactionDefaultsFromModelMetadata(model)
		out = append(out, map[string]any{"id": model.ID, "context": model.ContextWindow, "max_context": model.MaxContextWindow, "auto_limit": model.AutoCompactTokenLimit, "percent": model.EffectiveContextWindowPercent, "trigger": trigger, "target": target, "valid": valid})
	}
	return out
}

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
	out := map[string]any{"threshold_cases": nativeThresholdObservations(), "catalog_cases": nativeCatalogObservations(), "unicode_version": unicode.Version, "scalar_count": count, "scalar_sha256": hex.EncodeToString(hash.Sum(nil)), "catalog": catalog, "cases": cases, "retry": map[string]any{"found": []bool{first, second, third}, "requests_before_cooldown": before, "requests_after_cooldown": failed.Load()}}
	data, err := json.Marshal(out)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
