// SPDX-License-Identifier: GPL-3.0-only
package agents

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"testing"
)

func TestNativeKindsReference(t *testing.T) {
	names := []string{"", " ", "unknown-role", "Explore", "Plan", "general-purpose", "GENERAL-PURPOSE", "security_reviewer", "general purpose", "WRİTER", "ARCHİTECT", "ſecurity-reviewer", "\u200bexplore", "explore\ufeff", "explore\x00"}
	for _, kind := range BuiltInKinds() {
		names = append(names, string(kind), strings.ToUpper(string(kind)))
		for _, space := range []rune{'\t', '\n', '\v', '\f', '\r', ' ', 0x85, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000} {
			names = append(names, string(space)+strings.ToUpper(string(kind))+string(space))
		}
	}
	resolved := []map[string]any{}
	for _, name := range names {
		definition, ok := ResolveKind(name)
		parsed, parsedOK := ParseKind(name)
		resolved = append(resolved, map[string]any{"input": name, "definition": definition, "ok": ok, "parsed": parsed, "parsed_ok": parsedOK})
	}
	lists := [][]string{nil, {}, {"unknown-role"}, {"explore", "general-purpose"}, {"explore", "security-reviewer"}, {"WRİTER"}, {"Plan", "Plan"}}
	for _, name := range names[:37] {
		lists = append(lists, []string{name})
	}
	for _, kind := range BuiltInKinds() {
		lists = append(lists, []string{string(kind)}, []string{" \t" + strings.ToUpper(string(kind)) + " \n"})
	}
	allowed := []map[string]any{}
	kinds := append(BuiltInKinds(), Kind(""), Kind("unknown-role"), Kind("Explore"), Kind("Plan"), Kind("general-purpose"), Kind("executor "))
	for _, kind := range kinds {
		for _, list := range lists {
			allowed = append(allowed, map[string]any{"kind": kind, "allowed": list, "result": IsAllowed(kind, list)})
		}
	}
	codecs := []map[string]any{}
	add := func(typ, input string) {
		var value any
		if typ == "Kind" {
			value = new(Kind)
		} else {
			value = new(Definition)
		}
		err := json.Unmarshal([]byte(input), value)
		var fields any
		if typ == "Kind" {
			fields = string(*value.(*Kind))
		} else {
			d := value.(*Definition)
			fields = map[string]any{"kind": d.Kind, "readOnly": d.ReadOnly, "aliases": d.Aliases}
		}
		bytes, marshalErr := json.Marshal(value)
		codecs = append(codecs, map[string]any{"type": typ, "input_hex": hex.EncodeToString([]byte(input)), "ok": err == nil, "fields": fields, "marshal_ok": marshalErr == nil, "marshal_hex": hex.EncodeToString(bytes)})
	}
	for _, typ := range []string{"Kind", "Definition"} {
		for _, input := range []string{`null`, `{}`, `[]`, `true`, `false`, `0`, `1.5`, `""`, `"unknown-role"`, `"explore"`, `"\ud800"`, `"\u0000<&>"`, ``, `{`, `{} {}`, "\"" + string([]byte{0xff}) + "\""} {
			add(typ, input)
		}
	}
	for _, field := range []struct{ key, good string }{{"kind", `"explore"`}, {"readOnly", `true`}, {"aliases", `["a","b"]`}} {
		for _, input := range []string{`null`, `0`, `-0`, `1.5`, `true`, `false`, `"x"`, `""`, `{}`, `[]`, `[null]`, `["x",null]`, `[42]`} {
			add("Definition", fmt.Sprintf(`{%q:%s}`, field.key, input))
			add("Definition", fmt.Sprintf(`{%q:%s,%q:%s}`, field.key, field.good, field.key, input))
		}
		add("Definition", fmt.Sprintf(`{%q:%s}`, strings.ToUpper(field.key), field.good))
	}
	for _, input := range []string{
		`{"KIND":"executor","READONLY":true,"ALIAſES":["Plan"]}`,
		`{"kind":"explore","KIND":null,"readOnly":true,"READONLY":null}`,
		`{"aliases":["a","b"],"ALIASES":[null],"aliases":[null,null]}`,
		`{"aliases":["a","b"],"aliases":[],"aliases":[null,null]}`,
		`{"aliases":["a","b"],"aliases":null,"aliases":[null,null]}`,
		`{"aliases":["\ud800","<&>"]}`,
		`{"kind":"\ud800","unknown":{"deep":[1e9999]}}`,
		`{"kind":"\uZZZZ","kind":"explore"}`,
		`{"aliases":[42],"aliases":null}`,
	} {
		add("Definition", input)
	}
	add("Definition", "{\"kind\":\""+string([]byte{0xff})+"\",\"aliases\":[\""+string([]byte{0xff})+"\"]}")
	for _, depth := range []int{9999, 10000, 10001} {
		add("Definition", `{"unknown":`+strings.Repeat("[", depth-1)+`0`+strings.Repeat("]", depth-1)+`}`)
	}
	output, err := json.Marshal(map[string]any{"kinds": BuiltInKinds(), "definitions": Definitions(), "resolved": resolved, "allowed": allowed, "codecs": codecs})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("KINDS_OUTPUT"), output, 0600); err != nil {
		t.Fatal(err)
	}
}
