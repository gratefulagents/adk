package sandbox

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"
)

func TestNativeExpansionReference(t *testing.T) {
	env := map[string]string{"A": "alpha", "AB": "long", "_": "underscore", "1": "one", "10": "ten", "$": "dollar", "?": "question", "猫": "unicode", "A B": "spaces", "": "must-not-expand", "NESTED": "$A", "ZERO": "x\x00y"}
	values := []string{"", "$", "$$", "$A", "${A}", "${}", "${", "${A", "$A$AB", "$A-B", "${A B}", "$猫", "${猫}", "$10", "${10}", "$NESTED", "$ZERO", "${OPENAI_API_KEY}", "prefix\x00$A尾", "$${A}", "${A${AB}}", "${A\nB}", "$UNKNOWN", "$!"}
	env[string([]byte{255})] = string([]byte{254})
	env["A"+string([]byte{255})] = "raw-value"
	values = append(values, "${"+string([]byte{255})+"}", "$"+string([]byte{255})+"A", "${A"+string([]byte{255})+"}", "literal"+string([]byte{255})+"$A")
	for c := 0; c < 256; c++ {
		values = append(values, "$"+string([]byte{byte(c)})+"A", "${"+string([]byte{byte(c)})+"}tail")
	}
	cases := []map[string]string{}
	for _, value := range values {
		cases = append(cases, map[string]string{"input": hex.EncodeToString([]byte(value)), "output": hex.EncodeToString([]byte(ExpandSafe(value, env)))})
	}
	encodedEnv := map[string]string{}
	for key, value := range env {
		encodedEnv[hex.EncodeToString([]byte(key))] = hex.EncodeToString([]byte(value))
	}
	data, err := json.Marshal(map[string]any{"environment": encodedEnv, "cases": cases})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("EXPANSION_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
