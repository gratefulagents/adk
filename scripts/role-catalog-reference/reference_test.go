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
)

func TestNativeRoleCatalogReference(t *testing.T) {
	if strconv.IntSize != 64 {
		t.Fatalf("oracle requires IntSize 64, got %d", strconv.IntSize)
	}
	var cases []map[string]any
	seen := map[string]bool{}
	roleType := reflect.TypeOf(RoleSpec{})
	catalogType := reflect.TypeOf(RoleCatalog(nil))
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
		encoded, marshalErr := json.Marshal(value.Interface())
		marshalError := ""
		if marshalErr != nil {
			marshalError = marshalErr.Error()
		}
		cases = append(cases, map[string]any{
			"id": id, "type": typ.Name(), "input_hex": hex.EncodeToString([]byte(input)),
			"decode_ok": err == nil, "decode_error": decodeError,
			"decoded_fields": value.Interface(), "marshal_ok": marshalErr == nil,
			"marshal_error": marshalError, "marshal_hex": hex.EncodeToString(encoded),
		})
	}
	role := func(name, input string) {
		add(roleType, name, input)
		add(catalogType, "element/"+name, "["+input+"]")
	}
	for i, input := range []string{"{}", "null", "[]", `"role"`, "true", "42", "1.5", "", "{} {}", "{", "{\"Name\":}", "{\"Name\":\"x\",}"} {
		role(fmt.Sprintf("root-%d", i), input)
	}
	role("populated", `{"Name":" raw name ","Description":"<>&\u2028\u2029\n\t\"\\","Instructions":"do not trim\n","ToolAccess":"typo_UNNORMALIZED","ModelOverride":"model/x","FallbackModels":["one","","one"]}`)
	role("unknown", `{"unknown":{"nested":[true,null,1e999999,{"duplicate":1,"duplicate":2}]},"Name":"known","tool_access":"ignored","fallback_models":["ignored"]}`)
	role("unknown-invalid-json", `{"unknown":{"bad":01}}`)
	role("invalid-key", "{\"\xff\":null,\"Na\xffme\":\"ignored\",\"Name\":\"known\"}")
	role("surrogate-key", `{"\ud800":"ignored","\u004eame":"known"}`)
	role("unicode-fold", `{"Inſtructionſ":"folded","ToolAcceſſ":"unmodified","FallbacKModelſ":["kelvin"]}`)
	for i := 0; i < roleType.NumField(); i++ {
		field := roleType.Field(i)
		key := field.Name
		if field.Type.Kind() != reflect.String {
			continue
		}
		for j, value := range []string{`""`, "null", `"value"`, "false", "42", "1.2", "[]", "{}", `[null]`, `"<&>\u2028\u2029"`, `"\ud800"`, `"\udfff"`, `"\ud800\u0041"`, `"\ud800\ud800\udfff"`, `"\ud83d\ude00"`, `"é雪😀"`, "\"\xff\xc0\xaf\xe2\x82\"", `"\x41"`, "\"\x01\""} {
			role(fmt.Sprintf("%s/value-%d", key, j), fmt.Sprintf(`{%q:%s}`, key, value))
		}
		for j, pair := range [][2]string{{`"first"`, "null"}, {"null", `"second"`}, {`"first"`, `"second"`}, {"false", "null"}, {`"first"`, "false"}} {
			role(fmt.Sprintf("%s/duplicate-%d", key, j), fmt.Sprintf(`{%q:%s,%q:%s}`, key, pair[0], key, pair[1]))
		}
		role(key+"/casefold", fmt.Sprintf(`{%q:"first",%q:null,%q:"last"}`, strings.ToLower(key), strings.ToUpper(key), key))
	}
	for i, value := range []string{"null", "[]", `[null]`, `["",null,"x"]`, `["same","same"]`, `"model"`, "true", "42", "{}", "[true]", "[42]", "[{}]", "[[]]", `["\ud800","\ud83d\ude00","<>&\u2028\u2029"]`, "[\"\xff\xe2\x82\"]"} {
		role(fmt.Sprintf("fallback/value-%d", i), `{"FallbackModels":`+value+`}`)
	}
	for i, tail := range []string{
		`"FallbackModels":null`, `"FallbackModels":[]`, `"FallbackModels":[null]`,
		`"FallbackModels":[null,null,null]`,
		`"FallbackModels":["short"],"FallbackModels":[null,null,null]`,
		`"FallbackModels":[],"FallbackModels":[null,null]`,
		`"FallbackModels":null,"FallbackModels":[null,null]`,
		`"fallbackmodels":["new"],"FALLBACKMODELS":[null,null]`,
		`"FallbackModels":false,"FallbackModels":null`,
	} {
		role(fmt.Sprintf("fallback/repeated-%d", i), `{"FallbackModels":["old","tail"],`+tail+`}`)
	}
	role("fallback-null-then-empty", `{"FallbackModels":null,"FallbackModels":[]}`)
	role("fallback-empty-then-null", `{"FallbackModels":[],"FallbackModels":null}`)
	for _, depth := range []int{128, 9998, 9999, 10000} {
		role(fmt.Sprintf("unknown-depth-%d", depth), `{"unknown":`+strings.Repeat("[", depth)+"0"+strings.Repeat("]", depth)+`}`)
	}
	for i, input := range []string{"null", "[]", "{}", `{"Roles":[]}`, "true", "42", `"catalog"`, "[null]", "[null,{},null]", `[{}, {"Name":"same"}, {"Name":"same"}]`, `[{"FallbackModels":null},{"FallbackModels":[]},{"FallbackModels":[null]}]`, "[42]", "[false]", "[[]]", `["role"]`, "[{}] null", "[{},]", "["} {
		add(catalogType, fmt.Sprintf("root-%d", i), input)
	}
	var fields []map[string]string
	for i := 0; i < roleType.NumField(); i++ {
		field := roleType.Field(i)
		fields = append(fields, map[string]string{"name": field.Name, "type": field.Type.String(), "json_tag": field.Tag.Get("json")})
	}
	result := map[string]any{"schema_version": 1, "int_size": strconv.IntSize, "role_fields": fields, "catalog_kind": catalogType.Kind().String(), "cases": cases}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("ROLE_CATALOG_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
	t.Logf("recorded %d role/catalog cases at IntSize=%d", len(cases), strconv.IntSize)
}
