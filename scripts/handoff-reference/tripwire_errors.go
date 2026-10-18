package main

import (
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
)

func tripwireMessage(phase, name, tool string) string {
	switch phase {
	case "input":
		return (&agentsdk.InputGuardrailTripwireTriggered{GuardrailName: name}).Error()
	case "output":
		return (&agentsdk.OutputGuardrailTripwireTriggered{GuardrailName: name}).Error()
	case "tool input":
		return (&agentsdk.ToolInputGuardrailTripwireTriggered{GuardrailName: name, ToolName: tool}).Error()
	case "tool output":
		return (&agentsdk.ToolOutputGuardrailTripwireTriggered{GuardrailName: name, ToolName: tool}).Error()
	default:
		panic(phase)
	}
}

func generateTripwireErrors() ([]byte, error) {
	type observation struct {
		Phase   string `json:"phase"`
		Name    string `json:"name"`
		Tool    string `json:"tool"`
		Message string `json:"message"`
	}
	f := struct {
		SchemaVersion  int           `json:"schema_version"`
		SDKRevision    string        `json:"sdk_revision"`
		UnicodeVersion string        `json:"unicode_version"`
		ScalarCount    int           `json:"scalar_count"`
		ScalarSHA256   string        `json:"scalar_sha256"`
		Cases          []observation `json:"cases"`
	}{SchemaVersion: 1, SDKRevision: sdkRevision, UnicodeVersion: unicode.Version}
	h := sha256.New()
	for r := rune(0); r <= utf8.MaxRune; r++ {
		if !utf8.ValidRune(r) {
			continue
		}
		label := "a" + string(r) + "b"
		for _, message := range []string{
			tripwireMessage("input", label, ""),
			tripwireMessage("tool output", "guard", label),
		} {
			var size [4]byte
			binary.LittleEndian.PutUint32(size[:], uint32(len(message)))
			h.Write(size[:])
			h.Write([]byte(message))
		}
		f.ScalarCount++
	}
	f.ScalarSHA256 = hex.EncodeToString(h.Sum(nil))
	labels := []string{"", "plain", "a\"b\\c", "\x00\a\b\t\n\v\f\r\x1f\x7f", "\u0085\u00a0\u00ad\u200b\u2028\u2029", "é中😀\u0301", "\ue000\uffff\U000f0000\U0010ffff", " <>& / ", strings.Repeat("界", 1024)}
	for _, phase := range []string{"input", "output", "tool input", "tool output"} {
		for i, name := range labels {
			tool := labels[len(labels)-1-i]
			f.Cases = append(f.Cases, observation{phase, name, tool, tripwireMessage(phase, name, tool)})
		}
	}
	b, err := json.MarshalIndent(f, "", "  ")
	return append(b, '\n'), err
}
