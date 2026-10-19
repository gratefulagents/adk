// SPDX-License-Identifier: GPL-3.0-only
package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

func cursorFixture() object {
	inputs := []string{
		`{}`, `null`, `[]`, `1`, `"cursor"`, `true`,
		`{"message_id":0,"token":""}`, `{"message_id":-1,"token":"next"}`,
		`{"message_id":9223372036854775807}`, `{"message_id":-9223372036854775808}`,
		`{"message_id":9223372036854775808}`, `{"message_id":-9223372036854775809}`,
		`{"message_id":1.0}`, `{"message_id":1e0}`, `{"message_id":"1"}`, `{"message_id":true}`,
		`{"message_id":-0}`, `{"message_id":null,"token":null}`,
		`{"message_id":7,"message_id":null,"token":"keep","token":null}`,
		`{"message_id":7,"MESSAGE_ID":8,"token":"first","TOKEN":"last"}`,
		`{"message_id":7,"message_id":0,"token":"first","token":""}`,
		`{"message_id":"invalid","message_id":8}`, `{"token":1,"token":"last"}`,
		`{"MessageID":8,"Token":"tagged"}`, `{"meſsage_id":9,"toKen":"folded"}`,
		`{"token":"<>&\u2028\u2029\n\"\\"}`, `{"token":"\ud800"}`, `{"token":"\udc00"}`,
		`{"token":"\ud83d\ude80"}`, `{"token":[]}`, `{"token":{}}`,
		`{"unknown":{"token":"ignored"},"message_id":4}`, `{"unknown":1e9999}`,
		`{"message_id":1} {}`, `{"token":"unterminated}`, `{"token":false}`,
		"{\"token\":\"\xff\xc0\x80\"}",
	}
	for _, depth := range []int{127, 128, 9999, 10000} {
		inputs = append(inputs, `{"unknown":`+strings.Repeat("[", depth)+"0"+strings.Repeat("]", depth)+`}`)
	}
	cases := make([]object, 0, len(inputs))
	for index, input := range inputs {
		var cursor sdk.Cursor
		err := json.Unmarshal([]byte(input), &cursor)
		row := object{"name": fmt.Sprintf("cursor-%02d", index), "input_hex": hex.EncodeToString([]byte(input)), "accepted": err == nil}
		if err == nil {
			encoded, err := json.Marshal(cursor)
			if err != nil {
				panic(err)
			}
			row["wire_json"] = string(encoded)
			row["message_id"] = cursor.MessageID
			row["token"] = cursor.Token
		}
		cases = append(cases, row)
	}
	return object{"sdk_revision": pin, "schema_version": 1, "cases": cases}
}
