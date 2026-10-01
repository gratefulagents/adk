package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"strings"
	"sync"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	mode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
)

const pin = "1dc92b73900fac74dc357a938e4b5eee6392b418"

type object = map[string]any

type page struct {
	Messages []sdk.UserMessage
	Next     sdk.Cursor
}
type response struct {
	Items   []sdk.LLMRunItemSnapshot
	EndTurn *bool
	Error   string
	Usage   sdk.Usage
}
type decision struct {
	Approved bool
	Reason   string
}
type toolSpec struct {
	Name     string
	ReadOnly bool
	Pause    bool
	Output   string
}
type scenario struct {
	Name                                        string
	Runs                                        int
	Cursor                                      sdk.Cursor
	MessageLimit, MaxResumes                    int
	Permission                                  sdk.PermissionMode
	Directive                                   string
	Rules                                       []sdk.GuardrailRule
	Snapshot                                    *mode.TemplateSpec
	State                                       sdk.WorkingState
	Handoff                                     []sdk.LLMRunItemSnapshot
	Pages                                       []page
	Responses                                   []response
	BaseTools, FactoryTools                     []toolSpec
	Gate                                        bool
	Decisions                                   []decision
	Fail                                        map[string]int
	MaxTurns, SubAgentMaxTurns                  int
	Access                                      sdk.ToolAccessLevel
	Policy                                      *sdk.ToolPolicy
	AdditionalInstructions, WorkingStateContext string
}
type spy struct {
	s                        scenario
	mu                       sync.Mutex
	calls                    []json.RawMessage
	counts                   map[string]int
	page, response, decision int
}

func (s *spy) record(op string, data any) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.counts[op]++
	var err error
	if s.s.Fail[op] == s.counts[op] {
		err = fmt.Errorf("injected:%s", op)
	}
	event := object{"op": op, "args": data}
	if err != nil {
		event["error"] = err.Error()
	}
	b, e := json.Marshal(event)
	if e != nil {
		panic(e)
	}
	s.calls = append(s.calls, b)
	return err
}
func (s *spy) PermissionMode(context.Context) (sdk.PermissionMode, error) {
	return s.s.Permission, s.record("config.permission", nil)
}
func (s *spy) ModeDirective(context.Context) (string, error) {
	return s.s.Directive, s.record("config.directive", nil)
}
func (s *spy) GuardrailRules(context.Context) ([]sdk.GuardrailRule, error) {
	return s.s.Rules, s.record("config.guardrails", nil)
}
func (s *spy) ModeSnapshot(context.Context) (*mode.TemplateSpec, error) {
	return s.s.Snapshot, s.record("config.mode", nil)
}
func (s *spy) RoleCatalog(context.Context) (sdk.RoleCatalog, error) {
	return nil, s.record("config.roles", nil)
}
func restore(items []sdk.LLMRunItemSnapshot) []sdk.RunItem {
	out, err := sdk.RestoreRunItems(items, func(name string) *sdk.Agent { return &sdk.Agent{Name: name} })
	if err != nil {
		panic(err)
	}
	return out
}
func (s *spy) HandoffHistory(context.Context) ([]sdk.RunItem, error) {
	return restore(s.s.Handoff), s.record("config.handoff", nil)
}
func (s *spy) WorkingState(context.Context) (sdk.WorkingState, error) {
	return s.s.State, s.record("session.state", nil)
}
func (s *spy) LoadMessages(_ context.Context, c sdk.Cursor, limit int) ([]sdk.UserMessage, sdk.Cursor, error) {
	err := s.record("session.load", object{"cursor": c, "limit": limit})
	if s.page >= len(s.s.Pages) {
		panic("unscripted session load: " + s.s.Name)
	}
	p := s.s.Pages[s.page]
	s.page++
	return p.Messages, p.Next, err
}
func (s *spy) AppendRunItems(_ context.Context, items []sdk.RunItem) error {
	return s.record("session.append", object{"count": len(items), "items": sdk.SnapshotRunItems(items)})
}
func (s *spy) BuildTools(_ context.Context, base []sdk.Tool) ([]sdk.Tool, error) {
	err := s.record("factory.build", sdk.SnapshotTools(base))
	return append(append([]sdk.Tool(nil), base...), s.tools(s.s.FactoryTools)...), err
}
func (s *spy) PublishProgress(_ context.Context, p sdk.ProgressSnapshot) error {
	return s.record("status.progress", p)
}
func (s *spy) PublishTraceID(_ context.Context, id string) error {
	return s.record("status.trace_id", id)
}
func (s *spy) PublishFinalResult(_ context.Context, r *sdk.RunResult) error {
	return s.record("status.final", resultView(r))
}
func (s *spy) RunDir(context.Context) (string, error) {
	return "unused", s.record("trace.run_dir", nil)
}
func (s *spy) AppendCategory(_ context.Context, c, t string) error {
	return s.record("trace.category", object{"category": c, "text": t})
}
func (s *spy) WriteFile(_ context.Context, n string, b []byte) error {
	return s.record("trace.write", object{"name": n, "data": string(b)})
}
func (s *spy) Finalize(_ context.Context, r *sdk.RunResult) error {
	return s.record("trace.final", resultView(r))
}
func (s *spy) ApproveTool(_ context.Context, r sdk.ToolApprovalRequest) (bool, string, error) {
	err := s.record("gate.approve", object{"name": r.ToolName, "input": string(r.Input), "reason": r.Reason})
	if s.decision >= len(s.s.Decisions) {
		panic("unscripted approval: " + s.s.Name)
	}
	d := s.s.Decisions[s.decision]
	s.decision++
	return d.Approved, d.Reason, err
}

type tool struct {
	sdk.FunctionTool
	owner *spy
	spec  toolSpec
}

func (t *tool) Execute(_ context.Context, input json.RawMessage, workDir string) (sdk.ToolResult, error) {
	err := t.owner.record("tool.execute", object{"name": t.spec.Name, "input": string(input), "work_dir": workDir})
	return sdk.ToolResult{Content: t.spec.Output, ShouldPause: t.spec.Pause}, err
}
func (s *spy) tools(specs []toolSpec) []sdk.Tool {
	var out []sdk.Tool
	for _, ts := range specs {
		out = append(out, &tool{FunctionTool: sdk.FunctionTool{ToolName: ts.Name, Schema: json.RawMessage(`{"type":"object"}`), ReadOnly: ts.ReadOnly}, owner: s, spec: ts})
	}
	return out
}

type hooks struct {
	sdk.NoOpRunHooks
	owner *spy
}

func (h hooks) OnAgentStart(ctx *sdk.RunContext, a *sdk.Agent) {
	c := ctx.Config
	inputs, outputs := []string{}, []string{}
	for _, g := range c.ToolInputGuardrails {
		inputs = append(inputs, g.Name)
	}
	for _, g := range c.ToolOutputGuardrails {
		outputs = append(outputs, g.Name)
	}
	h.owner.record("runner.start", object{"agent": a.Name, "tools": sdk.SnapshotTools(a.Tools), "max_turns": c.MaxTurns, "subagent_max_turns": c.SubAgentMaxTurns, "access": c.ToolAccessLevel, "policy": c.ToolPolicy, "additional_instructions": c.AdditionalInstructions, "working_state_context": c.WorkingStateContext, "input_guardrails": inputs, "output_guardrails": outputs})
}
func (s *spy) GetResponse(_ context.Context, r sdk.ModelRequest) (*sdk.ModelResponse, error) {
	s.record("model.response", object{"model": r.Model, "instructions": r.Instructions, "input": sdk.SnapshotRunItems(r.Input), "tools": sdk.SnapshotTools(r.Tools), "prompt_cache_key": r.PromptCacheKey})
	if s.response >= len(s.s.Responses) {
		panic("unscripted model response: " + s.s.Name)
	}
	p := s.s.Responses[s.response]
	s.response++
	result := &sdk.ModelResponse{Items: restore(p.Items), EndTurn: p.EndTurn, Usage: p.Usage}
	if p.Error != "" {
		return result, errors.New(p.Error)
	}
	return result, nil
}
func (s *spy) StreamResponse(ctx context.Context, req sdk.ModelRequest) (*sdk.ModelStream, error) {
	resp, err := s.GetResponse(ctx, req)
	events := make(chan sdk.ModelStreamEvent, len(resp.Items)+2)
	done := make(chan *sdk.ModelResponse, 1)
	for i := range resp.Items {
		events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamItemDone, Item: &resp.Items[i]}
	}
	events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamComplete, Response: resp}
	if err != nil {
		events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamError, Error: err}
	}
	done <- resp
	close(events)
	close(done)
	return sdk.NewModelStream(events, done), nil
}
func (s *spy) GetRetryAdvice(error) *sdk.ModelRetryAdvice {
	return &sdk.ModelRetryAdvice{ShouldRetry: false}
}
func (s *spy) CalculateCost(sdk.Usage) float64 { return 0 }
func (s *spy) Provider() string                { return "host-reference" }
func resultView(r *sdk.RunResult) any {
	if r == nil {
		return nil
	}
	responses := []any{}
	for _, p := range r.RawResponses {
		responses = append(responses, object{"items": sdk.SnapshotRunItems(p.Items), "usage": p.Usage, "end_turn": p.EndTurn, "context_tokens": p.ContextTokens})
	}
	agent := ""
	if r.LastAgent != nil {
		agent = r.LastAgent.Name
	}
	return object{"final_output": r.FinalOutput, "final_text": r.FinalText(), "last_agent": agent, "new_items": sdk.SnapshotRunItems(r.NewItems), "final_history": sdk.SnapshotRunItems(r.FinalHistory), "responses": responses, "usage": r.Usage, "interrupted": r.IsInterrupted(), "interruption": r.Interruption, "interruptions": r.Interruptions, "last_response_id": r.LastResponseID, "tool_input_guardrails": r.ToolInputGuardrailResults, "tool_output_guardrails": r.ToolOutputGuardrailResults}
}
func errorView(err error) any {
	if err == nil {
		return nil
	}
	category := "runner"
	for _, prefix := range []string{"load permission mode", "load mode directive", "load guardrail rules", "compile guardrail rules", "load mode snapshot", "load working state", "build platform tools", "load handoff history", "load session messages", "append run items", "append denied approval items", "append approval items", "approve tool", "too many chat loop resumes", "finalize trace", "publish final result"} {
		if strings.HasPrefix(err.Error(), prefix) {
			category = prefix
			break
		}
	}
	return object{"category": category, "message": err.Error()}
}
func execute(spec scenario) object {
	s := &spy{s: spec, counts: map[string]int{}}
	a := &sdk.Agent{Name: "host-agent", Model: "fixture-model", Instructions: "base instructions", Tools: s.tools(spec.BaseTools)}
	cfg := sdk.RunConfig{MaxTurns: spec.MaxTurns, SubAgentMaxTurns: spec.SubAgentMaxTurns, ToolAccessLevel: spec.Access, ToolPolicy: spec.Policy, AdditionalInstructions: spec.AdditionalInstructions, WorkingStateContext: spec.WorkingStateContext, TracingDisabled: true, PromptCacheNamespace: "host-reference-fixed", Hooks: hooks{owner: s}}
	opts := sdk.ChatLoopOptions{Runner: sdk.NewRunnerWithModel(s), Agent: a, SessionStore: s, ConfigSource: s, PlatformToolFactory: s, TraceStore: s, RunStatusSink: s, RunConfig: cfg, Cursor: spec.Cursor, MessageLimit: spec.MessageLimit, MaxResumes: spec.MaxResumes}
	if spec.Gate {
		opts.ApprovalGate = s
	}
	loop := sdk.NewChatLoop(opts)
	runs := []any{}
	for i := 0; i < spec.Runs; i++ {
		start := len(s.calls)
		before := loop.Cursor()
		result, err := loop.Run(context.Background())
		runs = append(runs, object{"cursor_before": before, "cursor_after": loop.Cursor(), "calls": append([]json.RawMessage(nil), s.calls[start:]...), "result": resultView(result), "error": errorView(err)})
	}
	return object{"name": spec.Name, "spec": spec, "runs": runs, "original_agent_tools_after": sdk.SnapshotTools(a.Tools)}
}
func main() {
	conversation := flag.Bool("conversation", false, "emit the independent conversation and RunResult helper fixture")
	flag.Parse()
	var fixture object
	if *conversation {
		fixture = conversationFixture()
	} else {
		cases := []object{}
		for _, s := range scenarios() {
			cases = append(cases, execute(s))
		}
		fixture = object{"sdk_revision": pin, "schema_version": 1, "cases": cases}
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	enc.SetEscapeHTML(false)
	if err := enc.Encode(fixture); err != nil {
		panic(err)
	}
}
