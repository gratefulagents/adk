// SPDX-License-Identifier: GPL-3.0-only
package openai

import (
	"encoding/json"
	"os"
	"testing"
)

func TestNativeMetadataEndpoints(t *testing.T) {
	var out []map[string]string
	for _, host := range []string{"chatgpt.com", "api.chatgpt.com", "chatgpt.com:8443", "chatgpt.com:443", "chatgpt.com:", "api.githubcopilot.com:", "api.chatgpt.com:00443", "api.githubcopilot.com:443", "api.individual.githubcopilot.com:443", "api.githubcopilot.com", "api.individual.githubcopilot.com", "api.githubcopilot.com:8443", "api.individual.githubcopilot.com:8443", "API.GITHUBCOPILOT.COM", "api.githubcopilot.com.evil.test", "example.test"} {
		for _, path := range []string{"", "/custom", "/custom/responses", "/custom/chat/completions", "/v1"} {
			raw := "https://" + host + path
			out = append(out, map[string]string{"raw": raw, "endpoint": modelMetadataEndpoint(raw, nil)})
		}
	}
	for _, space := range "\t\n\v\f\r \u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000" {
		for _, endpoint := range []string{"https://example.test/custom", "https://chatgpt.com/backend-api/responses", "https://chatgpt.com:443/custom/chat/completions", "https://api.githubcopilot.com/custom"} {
			raw := string(space) + endpoint + string(space)
			out = append(out, map[string]string{"raw": raw, "endpoint": modelMetadataEndpoint(raw, nil)})
		}
	}
	for _, host := range []string{"example.test", "chatgpt.com", "api.githubcopilot.com", "chatgpt.com:443"} {
		for _, path := range []string{"/%76%31", "/custom%2Fresponses", "/custom%2fchat%2fcompletions", "/v1%2Fresponses", "/%E2%80%83", "/custom%20", "/custom%09", "/custom%2F%20", "/custom%3F%23", "/custom%25", "/%FF", "/%FF%E2%80%83", "/%E2%80%83%FF", "/%00", "/%5C", "/%21%27%28%29%2A", "/%24%26%2B%2C%3A%3B%3D%40", "/%CE%B2"} {
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
