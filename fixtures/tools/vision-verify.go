// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../fixtures/tools/vision-verify.go
package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/gratefulagents/sdk/pkg/agentsdk/tools/vision"
)

func main() {
	data, err := os.ReadFile("../../fixtures/tools/vision.json")
	if err != nil {
		panic(err)
	}
	var fixtures []struct {
		Name        string          `json:"name"`
		Path        string          `json:"path"`
		Data        []byte          `json:"data"`
		Arguments   json.RawMessage `json:"arguments"`
		MIME        string          `json:"mime"`
		Detail      string          `json:"detail"`
		Error       string          `json:"error"`
		ErrorPrefix string          `json:"error_prefix"`
	}
	if err := json.Unmarshal(data, &fixtures); err != nil {
		panic(err)
	}
	for _, configured := range []bool{false, true} {
		for _, fixture := range fixtures {
			root, err := os.MkdirTemp("", "adk-vision-")
			if err != nil {
				panic(err)
			}
			if fixture.Path != "" {
				if err := os.WriteFile(filepath.Join(root, fixture.Path), fixture.Data, 0600); err != nil {
					panic(err)
				}
			}
			tool := &vision.Tool{}
			if configured {
				tool.AnalyzeWithDetailFn = func(context.Context, []byte, string, string, string) (string, error) { panic("analyzer called") }
				tool.AnalyzeFn = func(context.Context, []byte, string, string) (string, error) { panic("analyzer called") }
			}
			result, err := tool.Execute(context.Background(), fixture.Arguments, root)
			if err != nil {
				panic(err)
			}
			if fixture.Error != "" || fixture.ErrorPrefix != "" {
				if !result.IsError || len(result.Images) != 0 || (fixture.Error != "" && result.Content != fixture.Error) || (fixture.ErrorPrefix != "" && !strings.HasPrefix(result.Content, fixture.ErrorPrefix)) {
					panic(fmt.Sprintf("%s: %+v", fixture.Name, result))
				}
			} else {
				var input struct {
					Prompt string `json:"prompt"`
				}
				if err := json.Unmarshal(fixture.Arguments, &input); err != nil {
					panic(err)
				}
				if result.IsError || result.Content != input.Prompt || len(result.Images) != 1 {
					panic(fmt.Sprintf("%s: %+v", fixture.Name, result))
				}
				image := result.Images[0]
				if image.MediaType != fixture.MIME || image.Detail != fixture.Detail || image.Data != base64.StdEncoding.EncodeToString(fixture.Data) {
					panic(fmt.Sprintf("%s: %+v", fixture.Name, image))
				}
			}
			if result.ShouldPause {
				panic("unexpected pause")
			}
			if err := os.RemoveAll(root); err != nil {
				panic(err)
			}
		}
	}
	fmt.Printf("verified %d pinned vision results (analyzer absent/present)\n", len(fixtures)*2)
}
