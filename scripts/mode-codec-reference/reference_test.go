// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"reflect"
	"strconv"
	"strings"
	"testing"

	sdkmode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
)

func TestNativeModeCodecReference(t *testing.T) {
	types := []struct {
		name   string
		typeOf reflect.Type
	}{
		{"ModelSettings", reflect.TypeOf(ModelSettings{})},
		{"RetryAdvice", reflect.TypeOf(ModelRetryAdvice{})},
		{"RoutingResolution", reflect.TypeOf(ModeRoutingResolution{})},
		{"ReasoningLevel", reflect.TypeOf(ModeReasoningLevel(""))},
		{"TextVerbosity", reflect.TypeOf(ModeTextVerbosity(""))},
		{"TemplateSpec", reflect.TypeOf(sdkmode.TemplateSpec{})},
		{"Constraints", reflect.TypeOf(sdkmode.Constraints{})},
		{"TemplateRouting", reflect.TypeOf(sdkmode.ModelRouting{})},
		{"TemplateRoleRouting", reflect.TypeOf(sdkmode.RoleModelRouting{})},
		{"RunnerRouting", reflect.TypeOf(ModeModelRouting{})},
		{"RunnerRoleRouting", reflect.TypeOf(ModeRoleModelRouting{})},
	}
	cases := []any{}
	metadata := map[string]any{}
	for _, target := range types {
		add := func(id, input string) {
			value := reflect.New(target.typeOf)
			err := json.Unmarshal([]byte(input), value.Interface())
			row := map[string]any{"id": target.name + "/" + id, "type": target.name, "input_hex": hex.EncodeToString([]byte(input)), "decode_ok": err == nil}
			if err != nil {
				row["decode_error"] = err.Error()
			} else {
				row["decoded_fields"] = value.Elem().Interface()
				if target.name == "ModelSettings" {
					row["settings_state"] = nativeSettingsState(value.Elem())
				} else if target.name == "RoutingResolution" {
					resolution := value.Elem().Interface().(ModeRoutingResolution)
					row["resolution_state"] = map[string]any{
						"Model": resolution.Model, "FallbackModels": resolution.FallbackModels,
						"ReasoningLevel": resolution.ReasoningLevel, "TextVerbosity": resolution.TextVerbosity,
						"ModelSettings": nativeSettingsState(reflect.ValueOf(resolution.ModelSettings)),
					}
				}
				encoded, marshalErr := json.Marshal(value.Interface())
				row["marshal_ok"] = marshalErr == nil
				if marshalErr != nil {
					row["marshal_error"] = marshalErr.Error()
				} else {
					row["marshal_hex"] = hex.EncodeToString(encoded)
				}
			}
			cases = append(cases, row)
		}
		for index, root := range []string{"null", "{}", "[]", "true", "42", `"mode"`, "{", "{} null", "{,}", `{"unknown":[null,true,{},[]]}`, `{"Name":"\ud800","unknown":"\udfff"}`} {
			add(fmt.Sprintf("root-%d", index), root)
		}
		if target.typeOf.Kind() == reflect.String {
			values := []string{"", "none", "minimal", "low", "medium", "high", "xhigh", "max", " HIGH ", "UNKNOWN/猫", "Kİſ", "<>&\u2028\u2029", "\x00\n\t", "false", "null", "42"}
			for index, value := range values {
				encoded, err := json.Marshal(value)
				if err != nil {
					t.Fatal(err)
				}
				add(fmt.Sprintf("scalar-%d", index), string(encoded))
			}
			for index, value := range []string{`"\ud800"`, `"\udfff"`, `"\ud83d\ude00"`, "\"\xff\xe2\x82\"", `"" ""`, "1.0", `{"value":"high"}`} {
				add(fmt.Sprintf("scalar-raw-%d", index), value)
			}
			metadata[target.name] = map[string]string{"kind": target.typeOf.Kind().String(), "type": target.typeOf.String()}
			continue
		}
		fields := []any{}
		for index := 0; index < target.typeOf.NumField(); index++ {
			field := target.typeOf.Field(index)
			fields = append(fields, map[string]string{"name": field.Name, "type": field.Type.String(), "json_tag": field.Tag.Get("json")})
			key := field.Name
			if tag := field.Tag.Get("json"); tag != "" {
				key = strings.Split(tag, ",")[0]
			}
			var initial string
			var values []string
			switch field.Type.Kind() {
			case reflect.String:
				initial = `"old"`
				values = []string{"null", `""`, `" raw "`, `"猫Kſ<>&\u2028\u2029"`, `"\ud800"`, `"\ud83d\ude00"`, "\"\xff\xe2\x82\"", "42", "false", "[]", "{}"}
			case reflect.Bool:
				initial = "true"
				values = []string{"null", "true", "false", "0", `"true"`, "[]", "{}"}
			case reflect.Int, reflect.Int64:
				initial = "7"
				values = []string{"null", "0", "-0", "-1", "9223372036854775807", "-9223372036854775808", "9223372036854775808", "-9223372036854775809", "1.0", "1e0", "01", "+1", `"3"`, "false", "[]", "{}"}
			case reflect.Slice:
				initial = `["old","tail"]`
				values = []string{"null", "[]", "[null]", "[null,null,null]", `[" raw ",""," raw "]`, `["\ud800","<>&"]`, "[\"\xff\"]", `"model"`, "42", "false", "{}", "[42]", "[false]", "[{}]", "[[]]"}
			case reflect.Map:
				initial = `{"keep":{"Model":"keep"},"same":{"Model":"old","FallbackModels":["old","tail"]}}`
				values = []string{"null", "{}", `{"new":null}`, `{"same":null}`, `{"same":{}}`, `{"same":{"ReasoningLevel":"high"}}`, `{"same":{"FallbackModels":[null]}}`, `{"same":{"Model":"first"},"same":{"TextVerbosity":"last"}}`, `{"x":{},"X":{},"猫":{"Model":" raw "},"<>&":null,"\ud800":{}}`, "{\"\xff\":{\"Model\":\"\xff\"}}", "[]", "true", "42", `"map"`, `{"x":[]}`, `{"x":42}`}
			case reflect.Struct:
				initial = `{"temperature":-0,"top_p":0.5,"max_tokens":7,"stop_sequences":["old","tail"]}`
				values = []string{"null", "{}", `{"temperature":null}`, `{"temperature":-0,"top_p":1e-7}`, `{"stop_sequences":[null]}`, `{"stop_sequences":[]}`, `{"stop_sequences":null}`, `{"thinking_budget":-9,"tool_choice":"raw"}`, "42", "true", "[]", `"settings"`}
			case reflect.Pointer:
				if field.Type.Elem().Kind() == reflect.Float64 {
					initial = "-0"
					values = []string{"null", "0", "-0", "1", "1.0", "1e0", "1e-324", "-1e-324", "5e-324", "-5e-324", "2.2250738585072014e-308", "1.7976931348623157e308", "1e309", "-1e309", "9007199254740993", "1.00000000000000011102230246251565404236316680908203125", "0.000001", "1e21", `"NaN"`, "true", "[]", "{}"}
				} else if field.Type.Elem().Kind() == reflect.Bool {
					initial = "true"
					values = []string{"null", "true", "false", "0", `"false"`, "[]", "{}"}
				} else if field.Name == "ModelRouting" {
					initial = `{"DefaultModel":"old","FallbackModels":["old","tail"],"RoleOverrides":{"keep":{"Model":"keep"}}}`
					values = []string{"null", "{}", `{"DefaultModel":null}`, `{"ReasoningLevel":" HIGH "}`, `{"FallbackModels":[null]}`, `{"RoleOverrides":{}}`, `{"RoleOverrides":null}`, "[]", "true", "42", `"pointer"`}
				} else {
					initial = `{"MaxTurns":7,"MaxRetries":3}`
					values = []string{"null", "{}", `{"MaxTurns":null}`, `{"MaxRetries":9}`, `{"MaxTurns":9223372036854775808}`, "[]", "true", "42", `"pointer"`}
				}
			default:
				t.Fatalf("unhandled field %s", field.Name)
			}
			for index, value := range values {
				add(fmt.Sprintf("%s/value-%d", field.Name, index), fmt.Sprintf(`{%q:%s}`, key, value))
				add(fmt.Sprintf("%s/repeated-%d", field.Name, index), fmt.Sprintf(`{%q:%s,%q:%s}`, key, initial, strings.ToLower(key), value))
			}
			add(field.Name+"/fold", fmt.Sprintf(`{%q:%s,%q:null}`, strings.ToUpper(key), initial, key))
			add(field.Name+"/null-reset", fmt.Sprintf(`{%q:%s,%q:null,%q:%s}`, key, initial, key, key, initial))
			if field.Type.Kind() == reflect.Slice {
				for index, reset := range []string{`["short"]`, "[]", "null"} {
					add(fmt.Sprintf("%s/slots-%d", field.Name, index), fmt.Sprintf(`{%q:%s,%q:%s,%q:[null,null,null]}`, key, initial, key, reset, key))
				}
			}
		}
		metadata[target.name] = fields
		for _, depth := range []int{128, 9998, 9999, 10000} {
			add(fmt.Sprintf("unknown-depth-%d", depth), `{"unknown":`+strings.Repeat("[", depth)+"0"+strings.Repeat("]", depth)+`}`)
		}
		if target.name == "RoutingResolution" {
			for _, prior := range cases {
				row := prior.(map[string]any)
				if row["type"] == "ModelSettings" {
					input, err := hex.DecodeString(row["input_hex"].(string))
					if err != nil {
						t.Fatal(err)
					}
					add("nested/"+row["id"].(string), `{"ModelSettings":`+string(input)+`}`)
				}
			}
			for index, middle := range []string{`{"stop_sequences":["short"]}`, `{"stop_sequences":[]}`, `{"stop_sequences":null}`, "null", "{}"} {
				add(fmt.Sprintf("value-backing-slots-%d", index), `{"ModelSettings":{"temperature":-0,"stop_sequences":["old","tail"]},"ModelSettings":`+middle+`,"ModelSettings":null,"ModelSettings":{"stop_sequences":[null,null,null]}}`)
			}
			add("value-invalid-before-null", `{"ModelSettings":{"temperature":1e309},"ModelSettings":null}`)
			add("value-field-fold", `{"MODELSETTINGS":{"temperature":-0},"ModelSettings":{"TEMPERATURE":null,"MAX_TOKENS":7},"modelsettings":{"ſtop_sequences":[]}}`)
		}
		if target.name == "TemplateSpec" {
			for index, middle := range []string{`{"FallbackModels":["short"]}`, `{"FallbackModels":[]}`, `{"FallbackModels":null}`, "null"} {
				add(fmt.Sprintf("pointer-backing-slots-%d", index), `{"ModelRouting":{"FallbackModels":["old","tail"]},"ModelRouting":`+middle+`,"ModelRouting":{"FallbackModels":[null,null,null]}}`)
			}
			add("pointer-invalid-before-null", `{"ModelRouting":{"FallbackModels":[42]},"ModelRouting":null}`)
			add("pointer-empty-spaces", `{"ModelRouting":{ },"ModelRouting":{ "DefaultModel":"raw" },"ModelRouting":{ },"ModelRouting":{"FallbackModels":[]}}`)
			add("pointer-reset-before-reallocation", `{"ModelRouting":{"FallbackModels":["old","tail"]},"ModelRouting":null,"ModelRouting":{},"ModelRouting":{"FallbackModels":[null,null]}}`)
			add("pointer-map-merge", `{"ModelRouting":{"RoleOverrides":{"keep":{"Model":"keep"},"same":{"Model":"old"}}},"modelrouting":{"RoleOverrides":{"same":{"ReasoningLevel":"high"}}}}`)
			add("pointer-casefold", `{"Conſtraints":{"MaxTurns":9},"CONSTRAINTS":{"MaxRetries":2},"ModelRouting":{"FallbacKModels":["raw"]}}`)
		}
	}
	merges := []any{}
	for _, baseJSON := range []string{
		`{}`,
		`{"temperature":0.75,"max_tokens":1000,"top_p":0.9,"tool_choice":"auto","parallel_tool_calls":true,"thinking_budget":500,"reasoning_effort":"high","text_verbosity":"low","stop_sequences":["base<>&"]}`,
		`{"temperature":-0,"max_tokens":-10,"top_p":-0,"parallel_tool_calls":false,"thinking_budget":-2,"stop_sequences":[]}`,
	} {
		for _, prior := range cases {
			row := prior.(map[string]any)
			if row["type"] != "ModelSettings" || row["decode_ok"] != true {
				continue
			}
			input, err := hex.DecodeString(row["input_hex"].(string))
			if err != nil {
				t.Fatal(err)
			}
			var base, override ModelSettings
			if err := json.Unmarshal([]byte(baseJSON), &base); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal(input, &override); err != nil {
				t.Fatal(err)
			}
			merged := base.Merge(override)
			encoded, err := json.Marshal(merged)
			if err != nil {
				t.Fatal(err)
			}
			merges = append(merges, map[string]any{"id": row["id"], "base": baseJSON, "override_hex": row["input_hex"], "state": nativeSettingsState(reflect.ValueOf(merged)), "marshal_hex": hex.EncodeToString(encoded)})
		}
	}
	nonfinite := []any{}
	for _, field := range []string{"Temperature", "TopP"} {
		for _, value := range []float64{math.NaN(), math.Inf(1), math.Inf(-1)} {
			settings := ModelSettings{}
			reflect.ValueOf(&settings).Elem().FieldByName(field).Set(reflect.ValueOf(&value))
			_, err := json.Marshal(settings)
			nonfinite = append(nonfinite, map[string]any{"field": field, "bits": fmt.Sprintf("%016x", math.Float64bits(value)), "marshal_ok": err == nil})
		}
	}
	output, err := json.Marshal(map[string]any{"settings_merges": merges, "nonfinite": nonfinite, "schema_version": 1, "int_size": strconv.IntSize, "types": metadata, "cases": cases, "constants": map[string]any{
		"ReasoningNone": ReasoningNone, "ReasoningMinimal": ReasoningMinimal, "ReasoningLow": ReasoningLow,
		"ReasoningMedium": ReasoningMedium, "ReasoningHigh": ReasoningHigh, "ReasoningXHigh": ReasoningXHigh, "ReasoningMax": ReasoningMax,
		"TextVerbosityLow": TextVerbosityLow, "TextVerbosityMedium": TextVerbosityMedium, "TextVerbosityHigh": TextVerbosityHigh,
	}})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("MODE_CODEC_OUTPUT"), output, 0600); err != nil {
		t.Fatal(err)
	}
}

func nativeSettingsState(value reflect.Value) map[string]any {
	state := map[string]any{}
	for i := 0; i < value.Type().NumField(); i++ {
		field := value.Field(i)
		name := value.Type().Field(i).Name
		if field.Kind() == reflect.Pointer && field.Type().Elem().Kind() == reflect.Float64 {
			if field.IsNil() {
				state[name] = nil
			} else {
				state[name] = fmt.Sprintf("%016x", math.Float64bits(field.Elem().Float()))
			}
		} else {
			state[name] = field.Interface()
		}
	}
	return state
}
