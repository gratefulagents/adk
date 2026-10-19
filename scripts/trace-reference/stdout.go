package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"os"
	"time"

	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/codes"
	"go.opentelemetry.io/otel/exporters/stdout/stdouttrace"
	"go.opentelemetry.io/otel/sdk/instrumentation"
	"go.opentelemetry.io/otel/sdk/resource"
	sdktrace "go.opentelemetry.io/otel/sdk/trace"
	"go.opentelemetry.io/otel/sdk/trace/tracetest"
	"go.opentelemetry.io/otel/trace"
)

func stdoutFixture() {
	var output bytes.Buffer
	exporter, err := stdouttrace.New(stdouttrace.WithWriter(&output), stdouttrace.WithPrettyPrint())
	must(err)
	traceID, err := trace.TraceIDFromHex("0102030405060708090a0b0c0d0e0f10")
	must(err)
	spanID, err := trace.SpanIDFromHex("0102030405060708")
	must(err)
	parentID, err := trace.SpanIDFromHex("0807060504030201")
	must(err)
	state, err := trace.ParseTraceState("vendor=opaque")
	must(err)
	parent := trace.NewSpanContext(trace.SpanContextConfig{TraceID: traceID, SpanID: parentID, TraceFlags: trace.FlagsSampled, TraceState: state, Remote: true})
	spanContext := trace.NewSpanContext(trace.SpanContextConfig{TraceID: traceID, SpanID: spanID, TraceFlags: trace.FlagsSampled, TraceState: state})
	start := time.Date(2025, 1, 2, 3, 4, 5, 123456000, time.UTC)
	res := resource.NewWithAttributes("https://opentelemetry.io/schemas/1.26.0", attribute.String("service.name", "fixture"))
	scope := instrumentation.Scope{Name: "gratefulagents/agent", Version: "test", SchemaURL: "scope/schema", Attributes: attribute.NewSet(attribute.String("scope.attr", "old"), attribute.String("scope.attr", "value"))}
	complete := tracetest.SpanStub{
		Name: "complete", SpanContext: spanContext, Parent: parent, SpanKind: trace.SpanKindClient,
		StartTime: start, EndTime: start.Add(time.Second),
		Attributes: []attribute.KeyValue{attribute.Bool("bool", true), attribute.Int64("int", -3), attribute.Float64("float", 0.5), attribute.String("string", "<value>&\u2028"), attribute.BoolSlice("bools", []bool{true, false}), attribute.Int64Slice("ints", []int64{1, 2}), attribute.Float64Slice("floats", []float64{0.25, 0.5}), attribute.StringSlice("strings", []string{"a", "b"})},
		Events:     []sdktrace.Event{{Name: "event", Time: start, Attributes: []attribute.KeyValue{attribute.String("event.attr", "value")}, DroppedAttributeCount: 1}},
		Links:      []sdktrace.Link{{SpanContext: parent, Attributes: []attribute.KeyValue{attribute.String("link.attr", "value")}, DroppedAttributeCount: 2}},
		Status:     sdktrace.Status{Code: codes.Error, Description: "failure"}, DroppedAttributes: 3, DroppedEvents: 4, DroppedLinks: 5, ChildSpanCount: 2,
		Resource: res, InstrumentationScope: scope,
	}
	root := tracetest.SpanStub{Name: "root", SpanContext: spanContext, SpanKind: trace.SpanKindInternal, StartTime: start, EndTime: start, Resource: res, InstrumentationScope: instrumentation.Scope{Name: "gratefulagents/agent"}}
	must(exporter.ExportSpans(context.Background(), tracetest.SpanStubs{complete, root}.Snapshots()))
	must(exporter.Shutdown(context.Background()))
	encoded := output.String()
	decoder := json.NewDecoder(&output)
	var records []any
	for {
		var record any
		err := decoder.Decode(&record)
		if err == io.EOF {
			break
		}
		must(err)
		records = append(records, record)
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(map[string]any{"records": records, "json": encoded}))
}
