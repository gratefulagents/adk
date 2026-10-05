package main

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
)

const sdkRevision = "1dc92b73900fac74dc357a938e4b5eee6392b418"

type agentRef struct {
	Name string
}

// Agent contains function fields unsupported by encoding/json, even when nil.
type encodedItem struct {
	agentsdk.RunItem
	Agent *agentRef
}

func encodeItems(items []agentsdk.RunItem) []encodedItem {
	if items == nil {
		return nil
	}
	out := make([]encodedItem, len(items))
	for i, item := range items {
		out[i].RunItem = item
		if item.Agent != nil {
			out[i].Agent = &agentRef{Name: item.Agent.Name}
		}
	}
	return out
}

type scenario struct {
	Name       string
	SourceOnly []string
	Input      []agentsdk.RunItem
	NewItems   []agentsdk.RunItem
}

type observation struct {
	Name       string        `json:"name"`
	SourceOnly []string      `json:"source_only"`
	Input      []encodedItem `json:"input"`
	NewItems   []encodedItem `json:"new_items"`
	Output     []encodedItem `json:"output"`
}

type fixture struct {
	SchemaVersion int           `json:"schema_version"`
	SDKRevision   string        `json:"sdk_revision"`
	Cases         []observation `json:"cases"`
}

func generate() ([]byte, error) {
	f := fixture{SchemaVersion: 1, SDKRevision: sdkRevision}
	for _, s := range scenarios() {
		reasons := s.SourceOnly
		if reasons == nil {
			reasons = []string{}
		}
		input, newItems := encodeItems(s.Input), encodeItems(s.NewItems)
		output := agentsdk.RemoveAllToolsHandoffInputFilter(s.Input, s.NewItems)
		f.Cases = append(f.Cases, observation{s.Name, reasons, input, newItems, encodeItems(output)})
	}
	b, err := json.MarshalIndent(f, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(b, '\n'), nil
}

func main() {
	var b []byte
	var err error
	switch {
	case len(os.Args) == 1:
		b, err = generate()
	case len(os.Args) == 2 && os.Args[1] == "catalog":
		b, err = generateCatalog()
	case len(os.Args) == 2 && os.Args[1] == "subagents":
		b, err = generateSubagents()
	case len(os.Args) == 2 && os.Args[1] == "final-summary":
		b, err = generateFinalSummary()
	default:
		err = fmt.Errorf("usage: handoff-reference [catalog|subagents|final-summary]")
	}
	if err == nil {
		_, err = os.Stdout.Write(b)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
