package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"slices"
	"testing"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
)

func TestFilterObservations(t *testing.T) {
	wantIndices := map[string][]int{
		"user_message": {0}, "assistant_message": {0}, "phased_messages": {0, 1},
		"compaction": {0}, "distinct_agent_history": {0, 1, 2, 3},
		"agent_named_system": {0}, "agent_named_developer": {0},
		"zero_value_item": {0}, "unknown_types": {0, 1},
		"mixed_sequence":        {1, 3, 5, 7, 9, 11},
		"native_mixed_sequence": {1, 3, 5, 6, 8, 9}, "new_items_ignored": {0, 2},
		"nil_input": {}, "empty_input": {}, "strip_tool_call": {}, "strip_tool_output": {},
		"strip_handoff_call": {}, "strip_handoff_output": {}, "strip_reasoning": {},
		"strip_tool_approval": {}, "all_six_stripped": {},
		"nil_input_nonempty_new_items": {}, "empty_input_nonempty_new_items": {},
	}
	for _, s := range scenarios() {
		t.Run(s.Name, func(t *testing.T) {
			indices, exists := wantIndices[s.Name]
			if !exists {
				t.Fatal("missing assertion")
			}
			beforeInput, beforeNew := slices.Clone(s.Input), slices.Clone(s.NewItems)
			got := agentsdk.RemoveAllToolsHandoffInputFilter(s.Input, s.NewItems)
			if got == nil || len(got) != len(indices) {
				t.Fatalf("expected nonnil result with %d items, got %#v", len(indices), got)
			}
			for i, index := range indices {
				if got[i] != s.Input[index] {
					t.Fatalf("result %d does not retain exact input item %d and its pointers", i, index)
				}
			}
			if !reflect.DeepEqual(s.Input, beforeInput) || !reflect.DeepEqual(s.NewItems, beforeNew) {
				t.Fatal("arguments mutated")
			}
			if !reflect.DeepEqual(got, agentsdk.RemoveAllToolsHandoffInputFilter(s.Input, nil)) {
				t.Fatal("second argument affected result")
			}
		})
	}
}

func TestSerializationBoundary(t *testing.T) {
	_, err := json.Marshal(agentsdk.RunItem{Agent: &agentsdk.Agent{Name: "alpha"}})
	var unsupported *json.UnsupportedTypeError
	if !errors.As(err, &unsupported) || unsupported.Type.Kind() != reflect.Func {
		t.Fatalf("expected SDK Agent function-field serialization failure, got %v", err)
	}
	for _, s := range scenarios() {
		for _, items := range [][]agentsdk.RunItem{s.Input, s.NewItems, agentsdk.RemoveAllToolsHandoffInputFilter(s.Input, s.NewItems)} {
			encoded := encodeItems(items)
			if (items == nil) != (encoded == nil) {
				t.Fatal("encoding changed slice nilness")
			}
			for i, item := range items {
				if encoded[i].RunItem != item {
					t.Fatal("encoding changed SDK item")
				}
				if item.Agent != nil && (encoded[i].Agent == nil || encoded[i].Agent.Name != item.Agent.Name) {
					t.Fatal("encoding changed agent label")
				}
				actual, err := json.Marshal(encoded[i])
				if err != nil {
					t.Fatal(err)
				}
				var view map[string]json.RawMessage
				if err := json.Unmarshal(actual, &view); err != nil {
					t.Fatal(err)
				}
				label, err := json.Marshal(encoded[i].Agent)
				if err != nil || !bytes.Equal(view["Agent"], label) {
					t.Fatal("Agent override was not serialized")
				}
				item.Agent = nil
				raw, err := json.Marshal(item)
				if err != nil {
					t.Fatal(err)
				}
				var direct map[string]json.RawMessage
				if err := json.Unmarshal(raw, &direct); err != nil {
					t.Fatal(err)
				}
				delete(view, "Agent")
				delete(direct, "Agent")
				if !reflect.DeepEqual(view, direct) {
					t.Fatal("non-Agent fields differ from direct SDK serialization")
				}
			}
		}
	}
}

func TestFixtureRepeatability(t *testing.T) {
	want, err := os.ReadFile("../../fixtures/handoff/sdk-handoff-filter.json")
	if err != nil {
		t.Fatal(err)
	}
	for range 2 {
		got, err := generate()
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(got, want) {
			t.Fatal("fixture differs from Go regeneration")
		}
	}
}
