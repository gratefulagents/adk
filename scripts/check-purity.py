#!/usr/bin/env python3
"""Check all-feature Cargo resolution, not just manifest spelling, for purity."""
import json
import subprocess

REUSABLE = {"adk", "adk-core", "adk-codec", "adk-runtime", "adk-security", "adk-sandbox", "adk-durable", "adk-project-state", "adk-mcp", "adk-providers", "adk-tools"}
FORBIDDEN = {"adk-platform", "adk-agent", "adk-harness", "kube", "kube-client", "kube-core", "kube-runtime", "k8s-openapi"}


def violations(metadata):
    packages = {p["id"]: p for p in metadata["packages"]}
    edges = {n["id"]: n["dependencies"] for n in metadata["resolve"]["nodes"]}
    failures = []
    for root, package in packages.items():
        if package["name"] not in REUSABLE:
            continue
        pending, seen = [(root, [package["name"]])], set()
        while pending:
            node, path = pending.pop()
            if node in seen:
                continue
            seen.add(node)
            if packages[node]["name"] in FORBIDDEN:
                failures.append(" -> ".join(path))
            pending.extend((dep, path + [packages[dep]["name"]]) for dep in edges[node])
    return failures


if __name__ == "__main__":
    data = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--all-features", "--format-version", "1"
    ]))
    failures = violations(data)
    if failures:
        raise SystemExit("Platform dependencies in reusable ADK:\n" + "\n".join(failures))
    print("Reusable ADK dependency closure is platform-free (all features).")
