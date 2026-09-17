#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Extract tool contracts from the pinned ledger, never from the live checkout."""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "docs/migration/ledger/sdk-v0.0.115/inventory.json"
OUTPUT = ROOT / "crates/adk-tools/src/manifest.json"


def generate():
    records = json.loads(LEDGER.read_text())["records"]["tools"]
    result = []
    for record in records:
        if not record["type"].startswith("pkg/agentsdk/tools/"):
            continue
        package = record["type"].split("/")[-1].split(".")[0]
        name, = record["literal_names"]
        family = {"search": "workspace-search", "fs": "workspace-filesystem",
                  "web": "web-fetch", "projectstate": "project-state"}.get(package, package)
        feature = name
        if package == "git":
            family, feature = {
                "attach_repository": ("attach-repository", "AttachRepository"),
                "create_pull_request": ("github-pull-request", "GitHubPullRequest"),
                "create_github_issue": ("github-issue", "GitHubIssue"),
            }[name]
        elif name in ("BashStart", "BashPoll", "BashKill"):
            family, feature = "async-shell", "AsyncShell"
        elif name == "Bash":
            family = "bash"
        elif name == "Terminal":
            family, feature = "interactive-terminal", "InteractiveTerminal"
        elif name == "think":
            family, feature = "think", "Think"
        elif package == "signal":
            family = "signals"
            feature = {"AskUserQuestion": "Signals.AskUserQuestion", "present_plan": "Signals.PresentPlan", "finish": "Signals.Finish"}.get(name, "ExtraTools")
        elif package == "projectstate":
            feature = "ProjectState.PrimeTool" if name == "prime_context" else "ProjectState.TaskTools" if name.startswith("task_") else "ProjectState.MemoryTools"
        elif package in ("skills", "memory"):
            feature = "ExtraTools"
        else:
            feature = {"list_files": "ListFiles", "read_file": "ReadFile", "glob": "Glob", "grep": "Grep", "AnalyzeImage": "Vision"}.get(name, name)
        methods = record["methods"]
        descriptions = methods["Description"][0]["literal_returns"]
        schemas = record["literal_schemas"]
        readonly = methods["IsReadOnly"][0]["returns"]
        assert methods["NeedsApproval"][0]["returns"] == ["false"]
        assert methods["TimeoutSeconds"][0]["returns"] == ["0"]
        variants = [("any", 0)]
        typename = record["type"].split(".")[-1]
        if name == "Browser":
            variants = [("read_only", 0), ("write", 1)]
        elif name in ("Write", "Edit", "Bash"):
            mode = "workspace_write" if typename.startswith("Workspace") else "read_only" if typename.startswith("ReadOnly") else "full_access"
            variants = [(mode, 0)]
        for mode, index in variants:
            definition = None
            if schemas:
                definition = dict(name=name, description=descriptions[index], input_schema=schemas[index],
                                  read_only=(mode == "read_only" if name == "Browser" else readonly == ["true"]), requires_approval=False)
            result.append(dict(name=name, family=family, feature=feature, mode=mode,
                               classification="host-only" if feature == "ExtraTools" else "runtime-built-in",
                               legacy=feature in ("ListFiles", "ReadFile", "Glob", "Grep", "LSP", "Bash", "Write", "Edit", "ApplyPatch", "Move", "Delete", "WebFetch", "AsyncShell", "Signals.AskUserQuestion", "Signals.PresentPlan", "Signals.Finish"),
                               read_only=mode == "read_only" if name == "Browser" else readonly == ["true"],
                               control_flow=name in ("finish", "save_plan", "get_plan"),
                               writes_git_remote=name in ("Terminal", "create_pull_request"),
                               definition=definition, source_type=record["type"], acceptance_id=record["acceptance_id"]))
    return json.dumps(sorted(result, key=lambda x: (x["name"], x["mode"])), indent=2, ensure_ascii=False) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    content = generate()
    if args.check:
        if OUTPUT.read_text() != content:
            raise SystemExit("tool manifest differs from the pinned ledger")
        print("tool manifest matches pinned ledger")
    else:
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        OUTPUT.write_text(content)
