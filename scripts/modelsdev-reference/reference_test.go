package modelsdev

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
	"unicode"
)

type nativeModelsTransport struct {
	calls  int
	body   io.ReadCloser
	status int
}

func (r *nativeModelsTransport) RoundTrip(_ *http.Request) (*http.Response, error) {
	r.calls++
	if r.status == 0 {
		r.status = 503
	}
	if r.body == nil {
		r.body = io.NopCloser(strings.NewReader("offline"))
	}
	return &http.Response{StatusCode: r.status, Body: r.body, Header: make(http.Header)}, nil
}

type nativeModelsReadError struct{}

func (nativeModelsReadError) Read([]byte) (int, error) { return 0, io.ErrUnexpectedEOF }

func TestNativeModelsDevReference(t *testing.T) {
	inputs := []string{sampleCatalog, `{}`, `null`, `[]`, `not json`, `{"p":null}`, `{"p":{"models":null}}`}
	for _, limit := range []string{`null`, `{}`, `{"context":1}`, `{"context":-1}`, `{"context":0}`, `{"context":-0}`, `{"context":1.0}`, `{"context":1e3}`, `{"context":9223372036854775807,"output":-9223372036854775808}`, `{"context":9223372036854775808}`, `{"context":true}`, `{"CONTEXT":99,"output":-7}`, `{"context":99,"context":null}`, `{"context":99,"output":4,"OUTPUT":null}`} {
		inputs = append(inputs, `{" P ":{"MODELS":{" M ":{"LIMIT":`+limit+`},"": {"limit":{"context":3}}}}}`)
	}
	inputs = append(inputs, `{"p":{"models":{"m":{"limit":{"context":8,"output":3},"LIMIT":{"context":null,"output":5}}}}}`, `{"p":{"models":{"m":{"limit":{"context":8}}},"MODELS":{"n":{"limit":{"context":16}}}}}`, `{"p":{"models":{"m":{"limit":{"context":8}},"m":{"limit":{"output":3}}}}}`, `{"p":{"models":{"m":{"limit":{"context":8}}},"models":null}}`, `{"p":{"models":{"m":{"limit":{"context":8}}}},"p":{"models":{"n":{"limit":{"context":16}}}}}`)
	inputs = append(inputs, `{"\ud800":{"models":{"\udc00":{"limit":{"context":3}}}}}`, `{"p":{"modelſ":{"m":{"limit":{"context":3}}}}}`)
	for _, depth := range []int{128, 9999, 10000} {
		inputs = append(inputs, `{"ignored":`+strings.Repeat("[", depth)+`0`+strings.Repeat("]", depth)+`,"p":{"models":{"m":{"limit":{"context":3}}}}}`)
	}
	parsed := []map[string]any{}
	for _, input := range inputs {
		catalog, err := ParseCatalog([]byte(input))
		parsed = append(parsed, map[string]any{"input": input, "ok": err == nil, "catalog": catalog})
	}
	catalog, err := ParseCatalog([]byte(sampleCatalog))
	if err != nil {
		t.Fatal(err)
	}
	catalog["openai"]["overflow"] = ModelLimits{ContextTokens: 9223372036854775807, OutputTokens: -1}
	r := NewResolver(WithCachePath(""))
	r.catalog = catalog
	r.fetchedAt = time.Now()
	resolved := []map[string]any{}
	for _, provider := range []string{"copilot", "github-copilot", " COPILOT ", "openai", "openai-oauth", "anthropic", "anthropic-oauth", "unknown", ""} {
		for _, model := range []string{"claude-fable-5", "copilot/claude-fable-5", " CLAUDE-FABLE-5 ", "claude-fable-5-20270101", "claude-fable", "gpt-5.3-codex", "gpt-5.3-codex-spark", "openai/gpt-5.4", "openai-oauth/gpt-5.4", "gpt-5.6-sol", "overflow", "", "unknown", "/gpt-5.4"} {
			limits, ok := r.Lookup(context.Background(), provider, model)
			trigger, target, thresholdOK := r.CompactionThresholds(context.Background(), provider, model)
			rt, rg, ro := r.CompactionResolverFunc(provider)(context.Background(), model)
			resolved = append(resolved, map[string]any{"provider": provider, "model": model, "limits": limits, "ok": ok, "thresholds": []any{trigger, target, thresholdOK}, "routed": []any{rt, rg, ro}})
		}
	}
	lower := map[string]string{}
	for c := rune(0); c <= unicode.MaxRune; c++ {
		if c >= 0xd800 && c <= 0xdfff {
			continue
		}
		v := string(c)
		if strings.ToLower(v) != v {
			lower[v] = strings.ToLower(v)
		}
	}
	cacheCases := []map[string]any{}
	freshness := []map[string]any{}
	cachePath := filepath.Join(t.TempDir(), "catalog.json")
	cacheResolver := NewResolver(WithCachePath(cachePath))
	cacheInputs := []string{`null`, `{}`, `{"catalog":null}`, `{"catalog":{}}`, `{"catalog":{"p":null}}`, `{"catalog":{"p":{"m":null}}}`, `{"catalog":{"p":{"m":{"context":100}}}}`, `{"fetched_at":null,"catalog":{"p":{"m":{"context":100}}}}`, `{"fetched_at":"","catalog":{"p":{"m":{"context":100}}}}`, `{"fetched_at":"2026-01-01T00:00:00Z","FETCHED_AT":null,"catalog":{"p":{"m":{"context":100}}},"CATALOG":{"q":{"n":{"context":200}}}}`, `{"catalog":{"p":{"m":{"context":100}}},"CATALOG":{"p":{"n":{"context":200}}}}`, `{"catalog":{"p":{"m":{"context":100}}},"catalog":null}`, `{"fetched_at":"\u0032026-01-01T00:00:00Z","catalog":{"p":{"m":{"context":100}}}}`}
	for _, stamp := range []string{"0000-01-01T00:00:00Z", "0001-01-01T00:00:00Z", "9999-12-31T23:59:59-24:60", "2026-01-02T3:04:05Z", "2026-01-02T03:04:05.12345678999Z", "2026-01-02T03:04:05,123Z", "2026-01-02T03:04:05+24:00", "2026-01-02T03:04:05+24:60", "2026-01-02T03:04:05-00:00", "2026-01-02T03:04:05+01:60", "2026-01-02T03:04:05+25:00", "2026-13-01T00:00:00Z", "2026-01-01T00:00:60Z"} {
		cacheInputs = append(cacheInputs, `{"fetched_at":"`+stamp+`","catalog":{"p":{"m":{"context":100,"output":-3}}}}`)
	}
	for _, input := range cacheInputs {
		if err := os.WriteFile(cachePath, []byte(input), 0600); err != nil {
			t.Fatal(err)
		}
		catalog, at, err := cacheResolver.loadDiskCache()
		cacheCases = append(cacheCases, map[string]any{"input": input, "ok": err == nil, "catalog": catalog, "timestamp": at.Format(time.RFC3339Nano), "seconds": at.Unix(), "nanos": at.Nanosecond()})
		for _, ttl := range []int64{0, 1, -1, -9223372036854775808, 9223372036854775807} {
			transport := &nativeModelsTransport{}
			resolver := NewResolver(WithCachePath(cachePath), WithHTTPClient(&http.Client{Transport: transport}), WithTTL(time.Duration(ttl)))
			limits, ok := resolver.Lookup(context.Background(), "p", "m")
			freshness = append(freshness, map[string]any{"input": input, "ttl": ttl, "hits": transport.calls, "limits": limits, "ok": ok})
		}

	}
	encodingCatalog := Catalog{"<provider>&\u2028": {"<model>&\u2029": ModelLimits{ContextTokens: 100, OutputTokens: -3}}}
	encodingTime := time.Date(2026, 1, 2, 3, 4, 5, 123456000, time.UTC)
	encodedCache, err := json.Marshal(diskCacheEnvelope{FetchedAt: encodingTime, Catalog: encodingCatalog})
	if err != nil {
		t.Fatal(err)
	}

	fetchCases := []map[string]any{}
	valid := `{"p":{"models":{"m":{"limit":{"context":100}}}}}`
	for _, name := range []string{"just_under_limit", "limit_after_valid", "truncated_json", "created_status", "reader_error"} {
		status := 200
		var body io.Reader = strings.NewReader(valid)
		switch name {
		case "just_under_limit":
			body = strings.NewReader(strings.Repeat(" ", maxCatalogBytes-len(valid)) + valid)
		case "limit_after_valid":
			body = strings.NewReader(valid + strings.Repeat(" ", maxCatalogBytes-len(valid)) + "junk")
		case "truncated_json":
			body = strings.NewReader(`{"pad":"` + strings.Repeat(" ", maxCatalogBytes) + `",` + valid[1:])
		case "created_status":
			status = 201
		case "reader_error":
			body = io.MultiReader(body, nativeModelsReadError{})
		}
		transport := &nativeModelsTransport{status: status, body: io.NopCloser(body)}
		resolver := NewResolver(WithCachePath(""), WithHTTPClient(&http.Client{Transport: transport}))
		limits, ok := resolver.Lookup(context.Background(), "p", "m")
		fetchCases = append(fetchCases, map[string]any{"name": name, "ok": ok, "limits": limits, "hits": transport.calls})
	}

	defaultResolver := NewResolver()
	defaults := map[string]any{"url": defaultResolver.url, "ttl": int64(defaultResolver.ttl), "timeout": int64(defaultResolver.client.Timeout), "catalog_nil": defaultResolver.catalog == nil, "zero_time": defaultResolver.fetchedAt.IsZero()}
	cachePaths := []map[string]any{}
	for _, env := range [][2]string{{"", ""}, {"/home/example", ""}, {"relative", ""}, {"/tmp/a/../b/", ""}, {"/home/example", "relative"}, {"", "/tmp/cache"}, {"/home/example", "/tmp/a/../cache/"}, {"/home/example", "/../../tmp/cache"}} {
		t.Setenv("HOME", env[0])
		t.Setenv("XDG_CACHE_HOME", env[1])
		r := NewResolver()
		cachePaths = append(cachePaths, map[string]any{"home": env[0], "xdg": env[1], "path": r.cachePath})
	}
	uncleanDir := t.TempDir()
	uncleanPath := uncleanDir + "/missing/../catalog.json"
	uncleanResolver := NewResolver(WithCachePath(uncleanPath))
	uncleanResolver.saveDiskCache(encodingCatalog)
	_, fileErr := os.Stat(filepath.Join(uncleanDir, "catalog.json"))
	_, dirErr := os.Stat(filepath.Join(uncleanDir, "missing"))

	result := map[string]any{"unclean_cache_path": map[string]any{"file_exists": fileErr == nil, "intermediate_exists": dirErr == nil}, "defaults": defaults, "cache_paths": cachePaths, "fetch_cases": fetchCases, "max_bytes": maxCatalogBytes, "encoding_catalog": encodingCatalog, "encoded_cache": string(encodedCache), "freshness": freshness, "cache": cacheCases, "parsed": parsed, "catalog": catalog, "resolved": resolved, "lower": lower}
	data, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("MODELSDEV_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
