// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"strconv"
	"strings"
	"testing"
)

var payloadRawType = reflect.TypeOf(json.RawMessage(nil))

func payloadFields(v reflect.Value) any {
	switch v.Kind() {
	case reflect.Struct:
		fields := map[string]any{}
		for i := 0; i < v.NumField(); i++ {
			field := v.Type().Field(i)
			if field.Type != payloadRawType {
				fields[strings.Split(field.Tag.Get("json"), ",")[0]] = payloadFields(v.Field(i))
			}
		}
		return fields
	case reflect.Slice:
		if v.IsNil() {
			return nil
		}
		fields := make([]any, v.Len())
		for i := range fields {
			fields[i] = payloadFields(v.Index(i))
		}
		return fields
	default:
		return v.Interface()
	}
}

func payloadSample(field reflect.StructField, populated bool) string {
	if field.Type == payloadRawType {
		if populated {
			return `{ "n": 1e+09, "n": -0, "s": "\ud800", "a": [true, null] }`
		}
		return `null`
	}
	switch field.Type.Kind() {
	case reflect.String:
		if populated {
			return `"value <&>\u2028\u2029\n\"\\"`
		}
		return `""`
	case reflect.Int:
		if populated {
			return "-7"
		}
		return "0"
	case reflect.Bool:
		if populated {
			return `true`
		}
		return `false`
	case reflect.Slice:
		if populated {
			return `[{"media_type":"image/png","data":"AQI=","detail":"high"},{"media_type":"image/jpeg","data":"/w==","detail":"low"}]`
		}
		return `[]`
	default:
		panic(field.Type.String())
	}
}

func TestNativePayloadReference(t *testing.T) {
	if strconv.IntSize != 64 {
		t.Fatalf("oracle requires IntSize 64, got %d", strconv.IntSize)
	}
	// Public RawMessage marshaling has a depth boundary independent of decoding
	// the containing payload (which adds an outer object level).
	for _, depth := range []int{10000, 10001} {
		raw := json.RawMessage(strings.Repeat("[", depth) + "0" + strings.Repeat("]", depth))
		for _, value := range []any{ToolCallData{Input: raw}, ToolApprovalData{Input: raw}, LLMOutputSchema{Schema: raw}, LLMToolSnapshot{InputSchema: raw}} {
			_, err := json.Marshal(value)
			if (err == nil) != (depth == 10000) {
				t.Fatalf("%T raw marshal depth %d: %v", value, depth, err)
			}
		}
	}
	var cases []map[string]any
	seen := map[string]bool{}
	add := func(typ reflect.Type, name, input string) {
		id := typ.Name() + "/" + name
		if seen[id] {
			t.Fatalf("duplicate case %s", id)
		}
		seen[id] = true
		value := reflect.New(typ)
		err := json.Unmarshal([]byte(input), value.Interface())
		decodeError := ""
		if err != nil {
			decodeError = err.Error()
		}
		var raw any
		var fields any
		if typ.Kind() == reflect.Struct {
			fields = payloadFields(value.Elem())
			for i := 0; i < typ.NumField(); i++ {
				if typ.Field(i).Type == payloadRawType {
					bytes := value.Elem().Field(i).Bytes()
					raw = map[string]any{"present": bytes != nil, "hex": hex.EncodeToString(bytes)}
				}
			}
		} else {
			fields = map[string]any{"value": value.Elem().Int()}
		}
		marshaled, marshalErr := json.Marshal(value.Interface())
		marshalError := ""
		if marshalErr != nil {
			marshalError = marshalErr.Error()
		}
		cases = append(cases, map[string]any{
			"id": id, "type": typ.Name(), "input_hex": hex.EncodeToString([]byte(input)),
			"decode_ok": err == nil, "decode_error": decodeError,
			"decoded_fields": fields, "raw_input": raw,
			"marshal_ok": marshalErr == nil, "marshal_error": marshalError,
			"marshal_hex": hex.EncodeToString(marshaled),
		})
	}
	types := []reflect.Type{
		reflect.TypeOf(MessageOutput{}), reflect.TypeOf(ImageAttachment{}),
		reflect.TypeOf(ToolCallData{}), reflect.TypeOf(ToolOutputData{}),
		reflect.TypeOf(HandoffCallData{}), reflect.TypeOf(HandoffOutputData{}),
		reflect.TypeOf(ReasoningData{}), reflect.TypeOf(CompactionData{}),
		reflect.TypeOf(ToolApprovalData{}), reflect.TypeOf(LLMOutputSchema{}), reflect.TypeOf(LLMToolSnapshot{}),
	}
	for _, typ := range types {
		var populated, zero []string
		for i := 0; i < typ.NumField(); i++ {
			field := typ.Field(i)
			key := strings.Split(field.Tag.Get("json"), ",")[0]
			populated = append(populated, strconv.Quote(key)+":"+payloadSample(field, true))
			zero = append(zero, strconv.Quote(key)+":"+payloadSample(field, false))
		}
		add(typ, "zero", `{}`)
		add(typ, "explicit_zero", "{"+strings.Join(zero, ",")+"}")
		add(typ, "populated", "{"+strings.Join(populated, ",")+"}")
		for i, input := range []string{`null`, `[]`, `[{}]`, `true`, `false`, `0`, `1.25`, `"text"`, ``, `{`, `{} {}`, `{"unknown":42}`, `{"unknown":{"arbitrary":[true,null,1e9999]}}`} {
			add(typ, fmt.Sprintf("root_%02d", i), input)
		}
		for i := 0; i < typ.NumField(); i++ {
			field := typ.Field(i)
			key := strings.Split(field.Tag.Get("json"), ",")[0]
			quoted := strconv.Quote(key)
			good := payloadSample(field, true)
			missing := append([]string{}, populated[:i]...)
			missing = append(missing, populated[i+1:]...)
			add(typ, key+"/missing", "{"+strings.Join(missing, ",")+"}")
			add(typ, key+"/populated", "{"+quoted+":"+good+"}")
			add(typ, key+"/null", "{"+quoted+":null}")
			add(typ, key+"/zero", "{"+quoted+":"+payloadSample(field, false)+"}")
			add(typ, key+"/upper", "{"+strconv.Quote(strings.ToUpper(key))+":"+good+"}")
			add(typ, key+"/go_name", "{"+strconv.Quote(field.Name)+":"+good+"}")
			add(typ, key+"/duplicate_null", "{"+quoted+":"+good+","+quoted+":null}")
			add(typ, key+"/duplicate_zero", "{"+quoted+":"+good+","+quoted+":"+payloadSample(field, false)+"}")
			add(typ, key+"/duplicate_folded", "{"+quoted+":"+payloadSample(field, false)+","+strconv.Quote(strings.ToUpper(key))+":"+good+"}")
			if strings.ContainsAny(key, "ks") {
				folded := strings.NewReplacer("k", "K", "s", "ſ").Replace(key)
				add(typ, key+"/unicode_fold", "{"+strconv.Quote(folded)+":"+good+"}")
			}
			for j, candidate := range []string{`42`, `1.5`, `true`, `"string"`, `{}`, `[]`} {
				add(typ, fmt.Sprintf("%s/type_%d", key, j), "{"+quoted+":"+candidate+"}")
				add(typ, fmt.Sprintf("%s/earlier_type_%d", key, j), "{"+quoted+":"+candidate+","+quoted+":"+good+"}")
			}
			if field.Type.Kind() == reflect.Int {
				for j, value := range []string{"-0", "9223372036854775807", "-9223372036854775808", "9223372036854775808", "-9223372036854775809", "1.0", "1e0"} {
					add(typ, fmt.Sprintf("%s/integer_%d", key, j), "{"+quoted+":"+value+"}")
				}
			}
			if field.Type.Kind() == reflect.String {
				for j, text := range []string{`"\ud800"`, `"\udfff"`, `"\ud800\udc00"`, `"\ud800\ud800\udc00"`, `"\\ud800"`, `"\uDFFF\uD800"`, `"\u0000\b\f\r\t/é😀<&>"`, `"\ud800\uZZZZ"`} {
					add(typ, fmt.Sprintf("%s/unicode_%d", key, j), "{"+quoted+":"+text+"}")
				}
				for j, invalid := range []string{string([]byte{0xff}), string([]byte{0xe2, 0x82}), string([]byte{0xed, 0xa0, 0x80}), string([]byte{0xf0, 0x80, 0x80, 0x80})} {
					add(typ, fmt.Sprintf("%s/invalid_utf8_%d", key, j), "{"+quoted+":\""+invalid+"\"}")
				}
			}
		}
		for _, depth := range []int{9999, 10000, 10001} {
			add(typ, fmt.Sprintf("unknown_depth_%d", depth), `{"unknown":`+strings.Repeat("[", depth-1)+`0`+strings.Repeat("]", depth-1)+`}`)
		}
		add(typ, "unknown_invalid_utf8", "{\"unknown\":\""+string([]byte{0xff})+"\"}")
		add(typ, "invalid_utf8_key", "{\""+string([]byte{0xff})+"\":true}")
	}
	for _, typ := range []reflect.Type{reflect.TypeOf(MessageOutput{}), reflect.TypeOf(ToolOutputData{})} {
		for i, input := range []string{
			`{"images":[null,{}]}`,
			`{"images":[false]}`, `{"images":[42]}`, `{"images":["x"]}`, `{"images":[[]]}`,
			`{"images":[{"media_type":42,"data":"ok"}]}`,
			`{"images":[{"MEDIA_TYPE":"image/png","DATA":"x","DETAIL":"high"}]}`,
			`{"imageſ":[{"media_type":"image/png"}]}`,
			`{"images":[{"media_type":"first","data":"a","detail":"high"}],"images":[{"data":"b"}]}`,
			`{"images":[{"media_type":"first","data":"a"},{"media_type":"tail","data":"z"}],"images":[null],"images":[{"detail":"low"},null]}`,
			`{"images":[{"media_type":"first"},{"media_type":"tail"}],"images":[{}],"images":[{},{}]}`,
			`{"images":[{"media_type":"first"},{"media_type":"tail"}],"images":[{}],"images":[{},{},{}]}`,
			`{"images":[{"media_type":"first"}],"images":[],"images":[{"data":"b"}]}`,
			`{"images":[{"media_type":"first"}],"images":null,"images":[{"data":"b"}]}`,
			`{"images":[{"media_type":"first"}],"IMAGES":[null]}`,
			`{"images":[false],"images":[{"data":"b"}]}`,
			`{"images":false,"images":[{"data":"b"}]}`,
			`{"images":[{"media_type":"a","MEDIA_TYPE":null,"data":"a","DATA":"b"}]}`,
		} {
			add(typ, fmt.Sprintf("image_reuse_%02d", i), input)
		}
	}
	for _, typ := range []reflect.Type{reflect.TypeOf(ToolCallData{}), reflect.TypeOf(ToolApprovalData{}), reflect.TypeOf(LLMOutputSchema{}), reflect.TypeOf(LLMToolSnapshot{})} {
		field := "input"
		if typ == reflect.TypeOf(LLMOutputSchema{}) {
			field = "schema"
		}
		if typ == reflect.TypeOf(LLMToolSnapshot{}) {
			field = "input_schema"
		}
		rawAdd := func(name, input string) {
			input = strings.ReplaceAll(input, `"input"`, strconv.Quote(field))
			input = strings.ReplaceAll(input, `"INPUT"`, strconv.Quote(strings.ToUpper(field)))
			add(typ, name, input)
		}
		for i, raw := range []string{
			`null`, `true`, `false`, `0`, `-0`, `-0.00`, `1e+09`, `1E-9`, `1.2300`,
			`9007199254740993`, `9223372036854775808`, `1e999999`, strings.Repeat("9", 400),
			`"\ud800"`, `"\udfff"`, `"\ud800\udc00"`, `"\\ud800"`, `"\u0061\/\u003c<&>\u2028"`,
			`[ 1, 2 , { "x" : 1, "x": 2 } ]`, `{ "b": 1e+09, "a": -0, "b": 1.2300 }`,
			`{ "\ud800": "\udfff", "n":1e999999 }`,
		} {
			rawAdd(fmt.Sprintf("raw_lexical_%02d", i), "{\"input\": \n\t"+raw+" \n}")
		}
		for i, input := range []string{
			`{"input":{"a":1},"INPUT": { "b": -0 }}`,
			`{"input":{"a":1},"input":null}`,
			`{"input":null,"input": { "a" : 1e+09 }}`,
			`{"input":true,"input":false,"input":[ 1, 2 ]}`,
			`{"input":01,"input":{}}`, `{"input":"\uZZZZ","input":{}}`,
			`{"input":NaN}`, `{"input":Infinity}`, `{"input":-Infinity}`,
		} {
			rawAdd(fmt.Sprintf("raw_duplicate_or_invalid_%02d", i), input)
		}
		for j, invalid := range []string{string([]byte{0xff}), string([]byte{0xe2, 0x82}), string([]byte{0xed, 0xa0, 0x80}), string([]byte{0xf0, 0x80, 0x80, 0x80})} {
			rawAdd(fmt.Sprintf("raw_invalid_utf8_%d", j), "{\"input\":\""+invalid+"\"}")
			rawAdd(fmt.Sprintf("raw_invalid_utf8_key_%d", j), "{\"input\":{\""+invalid+"\":\""+invalid+"\"}}")
		}
		for _, depth := range []int{9999, 10000, 10001} {
			rawAdd(fmt.Sprintf("raw_depth_%d", depth), `{"input":`+strings.Repeat("[", depth-1)+`0`+strings.Repeat("]", depth-1)+`}`)
		}
	}
	enumType := reflect.TypeOf(RunItemType(0))
	for i, input := range []string{
		`0`, `1`, `2`, `3`, `4`, `5`, `6`, `7`, `8`, `99`, `-1`, `-99`, `-0`,
		`9223372036854775807`, `-9223372036854775808`, `9223372036854775808`, `-9223372036854775809`,
		`18446744073709551615`, `1.0`, `0.0`, `-0.0`, `1e0`, `1E+0`, `1e999999`, `0e0`,
		`null`, `"1"`, `"message"`, `true`, `false`, `[]`, `{}`, ``, `01`, `+1`, `NaN`, `1 2`,
	} {
		add(enumType, fmt.Sprintf("value_%02d", i), input)
	}
	encoding := []map[string]any{}
	for _, raw := range []json.RawMessage{nil, {}, json.RawMessage(`null`), json.RawMessage(` { "b":1e+09, "a":-0, "b":1.00 } `), json.RawMessage("\"\xff\""), json.RawMessage(` `), json.RawMessage(`[1,]`)} {
		for _, value := range []any{LLMOutputSchema{Schema: raw}, LLMToolSnapshot{InputSchema: raw}} {
			encoded, err := json.Marshal(value)
			encoding = append(encoding, map[string]any{"type": reflect.TypeOf(value).Name(), "raw_nil": raw == nil, "raw_hex": hex.EncodeToString(raw), "marshal_ok": err == nil, "marshal_hex": hex.EncodeToString(encoded)})
		}
	}
	result := map[string]any{
		"schema_version": 1, "int_size": strconv.IntSize, "cases": cases, "snapshot_encode_cases": encoding,
		"enum_values": map[string]RunItemType{
			"RunItemMessage": RunItemMessage, "RunItemToolCall": RunItemToolCall,
			"RunItemToolOutput": RunItemToolOutput, "RunItemHandoffCall": RunItemHandoffCall,
			"RunItemHandoffOutput": RunItemHandoffOutput, "RunItemReasoning": RunItemReasoning,
			"RunItemToolApproval": RunItemToolApproval, "RunItemCompaction": RunItemCompaction,
		},
	}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("PAYLOAD_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
	t.Logf("recorded %d payload cases at IntSize=%d", len(cases), strconv.IntSize)
}
