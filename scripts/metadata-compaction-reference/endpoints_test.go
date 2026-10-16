// SPDX-License-Identifier: GPL-3.0-only
package openai

import (
	"encoding/json"
	"os"
	"testing"
)

func TestNativeMetadataEndpoints(t *testing.T) {
	var out []map[string]string
	for _, host := range []string{"chatgpt.com", "api.chatgpt.com", "chatgpt.com:8443", "api.githubcopilot.com", "api.individual.githubcopilot.com", "api.githubcopilot.com:8443", "api.individual.githubcopilot.com:8443", "API.GITHUBCOPILOT.COM", "api.githubcopilot.com.evil.test", "example.test"} {
		for _, path := range []string{"", "/custom", "/custom/responses", "/custom/chat/completions", "/v1"} {
			raw := "https://" + host + path
			out = append(out, map[string]string{"raw": raw, "endpoint": modelMetadataEndpoint(raw, nil)})
		}
	}
	data, err := json.Marshal(out)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("METADATA_ENDPOINT_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
