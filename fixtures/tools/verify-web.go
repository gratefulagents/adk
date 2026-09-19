// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../fixtures/tools/verify-web.go ../../target/debug/examples/web_replay
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tools/web"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"strconv"
	"strings"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func main() {
	data, err := os.ReadFile("../../fixtures/tools/web.json")
	must(err)
	var cases []struct {
		Body        string         `json:"body"`
		ContentType string         `json:"content_type"`
		Status      int            `json:"status"`
		Repeat      int            `json:"repeat"`
		Redirect    string         `json:"redirect"`
		Arguments   map[string]any `json:"arguments"`
	}
	must(json.Unmarshal(data, &cases))
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("User-Agent") != "gratefulagents-bot/1.0" || r.Header.Get("Accept-Encoding") != "" {
			panic("unexpected request headers")
		}
		index, err := strconv.Atoi(strings.TrimPrefix(r.URL.Path, "/case/"))
		must(err)
		test := cases[index]
		if test.ContentType != "" {
			w.Header().Set("Content-Type", test.ContentType)
		} else {
			w.Header().Set("Content-Type", "text/plain")
		}
		if test.Redirect != "" {
			w.Header().Set("Location", test.Redirect)
		}
		if test.Status > 0 {
			w.WriteHeader(test.Status)
		}
		body := test.Body
		if test.Repeat > 0 {
			body = strings.Repeat(body, test.Repeat)
		}
		_, _ = w.Write([]byte(body))
	}))
	defer server.Close()
	command := exec.Command(os.Args[1])
	command.Stderr = os.Stderr
	stdin, err := command.StdinPipe()
	must(err)
	stdout, err := command.StdoutPipe()
	must(err)
	must(command.Start())
	defer func() { _ = command.Process.Kill(); _ = command.Wait() }()
	encoder := json.NewEncoder(stdin)
	scanner := bufio.NewScanner(stdout)
	scanner.Buffer(make([]byte, 65536), 8<<20)
	tool := &web.FetchTool{AllowPrivateNetworkURLs: true}
	for index, test := range cases {
		arguments := test.Arguments
		if arguments == nil {
			arguments = map[string]any{}
		}
		arguments["url"] = fmt.Sprintf("%s/case/%d", server.URL, index)
		raw, err := json.Marshal(arguments)
		must(err)
		expected, err := tool.Execute(context.Background(), raw, "")
		must(err)
		// Byte pagination may split UTF-8; compare the SDK's model-facing JSON string.
		encoded, err := json.Marshal(expected.Content)
		must(err)
		must(json.Unmarshal(encoded, &expected.Content))
		must(encoder.Encode(arguments))
		if !scanner.Scan() {
			panic("Rust replay stopped")
		}
		var actual struct {
			Content []struct {
				Text string `json:"text"`
			} `json:"content"`
			IsError     bool `json:"is_error"`
			ShouldPause bool `json:"should_pause"`
		}
		must(json.Unmarshal(scanner.Bytes(), &actual))
		if actual.IsError != expected.IsError || actual.ShouldPause != expected.ShouldPause || len(actual.Content) != 1 || actual.Content[0].Text != expected.Content {
			panic(fmt.Sprintf("case %d: Go=%q error=%v Rust=%s", index, expected.Content, expected.IsError, scanner.Text()))
		}
	}
	fmt.Printf("%d SDK WebFetch HTTP and HTML differential cases verified\n", len(cases))
}
