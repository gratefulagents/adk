// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
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
		fields := []any{}
		for index := 0; index < target.typeOf.NumField(); index++ {
			field := target.typeOf.Field(index)
			fields = append(fields, map[string]string{"name": field.Name, "type": field.Type.String(), "json_tag": field.Tag.Get("json")})
			var initial string
			var values []string
			switch field.Type.Kind() {
			case reflect.String:
				initial = `"old"`
				values = []string{"null", `""`, `" raw "`, `"猫Kſ<>&\u2028\u2029"`, `"\ud800"`, `"\ud83d\ude00"`, "\"\xff\xe2\x82\"", "42", "false", "[]", "{}"}
			case reflect.Bool:
				initial = "true"
				values = []string{"null", "true", "false", "0", `"true"`, "[]", "{}"}
			case reflect.Int:
				initial = "7"
				values = []string{"null", "0", "-0", "-1", "9223372036854775807", "-9223372036854775808", "9223372036854775808", "-9223372036854775809", "1.0", "1e0", "01", "+1", `"3"`, "false", "[]", "{}"}
			case reflect.Slice:
				initial = `["old","tail"]`
				values = []string{"null", "[]", "[null]", "[null,null,null]", `[" raw ",""," raw "]`, `["\ud800","<>&"]`, "[\"\xff\"]", `"model"`, "42", "false", "{}", "[42]", "[false]", "[{}]", "[[]]"}
			case reflect.Map:
				initial = `{"keep":{"Model":"keep"},"same":{"Model":"old","FallbackModels":["old","tail"]}}`
				values = []string{"null", "{}", `{"new":null}`, `{"same":null}`, `{"same":{}}`, `{"same":{"ReasoningLevel":"high"}}`, `{"same":{"FallbackModels":[null]}}`, `{"same":{"Model":"first"},"same":{"TextVerbosity":"last"}}`, `{"x":{},"X":{},"猫":{"Model":" raw "},"<>&":null,"\ud800":{}}`, "{\"\xff\":{\"Model\":\"\xff\"}}", "[]", "true", "42", `"map"`, `{"x":[]}`, `{"x":42}`}
			case reflect.Pointer:
				if field.Name == "ModelRouting" {
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
				add(fmt.Sprintf("%s/value-%d", field.Name, index), fmt.Sprintf(`{%q:%s}`, field.Name, value))
				add(fmt.Sprintf("%s/repeated-%d", field.Name, index), fmt.Sprintf(`{%q:%s,%q:%s}`, field.Name, initial, strings.ToLower(field.Name), value))
			}
			add(field.Name+"/fold", fmt.Sprintf(`{%q:%s,%q:null}`, strings.ToUpper(field.Name), initial, field.Name))
			add(field.Name+"/null-reset", fmt.Sprintf(`{%q:%s,%q:null,%q:%s}`, field.Name, initial, field.Name, field.Name, initial))
			if field.Type.Kind() == reflect.Slice {
				for index, reset := range []string{`["short"]`, "[]", "null"} {
					add(fmt.Sprintf("%s/slots-%d", field.Name, index), fmt.Sprintf(`{%q:%s,%q:%s,%q:[null,null,null]}`, field.Name, initial, field.Name, reset, field.Name))
				}
			}
		}
		metadata[target.name] = fields
		for _, depth := range []int{128, 9998, 9999, 10000} {
			add(fmt.Sprintf("unknown-depth-%d", depth), `{"unknown":`+strings.Repeat("[", depth)+"0"+strings.Repeat("]", depth)+`}`)
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
	output, err := json.Marshal(map[string]any{"schema_version": 1, "int_size": strconv.IntSize, "types": metadata, "cases": cases})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("MODE_CODEC_OUTPUT"), output, 0600); err != nil {
		t.Fatal(err)
	}
}
