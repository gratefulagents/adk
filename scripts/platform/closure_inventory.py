#!/usr/bin/env python3
"""All-build-tags local import closure; no Go execution or source writes."""
import collections
import hashlib
import json
import pathlib
import re
import subprocess

PLATFORM = "github.com/gratefulagents/gratefulagents"
SDK = "github.com/gratefulagents/sdk"
ROOTS = {
    PLATFORM: pathlib.Path("repos/gratefulagents"),
    SDK: pathlib.Path("repos/sdk"),
}
PINS = {
    PLATFORM: "08e65c970830f05042c251bcbb46ec6a9e3719b9",
    SDK: "1dc92b73900fac74dc357a938e4b5eee6392b418",
}
SEEDS = [PLATFORM + "/cmd/agent", PLATFORM + "/internal/tools"]
IMPORT_DECL = re.compile(
    r'(?m)^import\s*\([\s\S]*?^\)'
    r'|^import[ \t]+(?:[\w.]+[ \t]+)?"[^"]+"'
)

# Package-level boundary classification. Shared packages can contain both
# contract definitions and implementations; inclusion is not a port mandate.
CONTROL_PLANE = {
    "internal/auth", "internal/controller/triggers",
    "internal/githubapp", "internal/linear",
}
WORKER = {
    "cmd/agent", "internal/agentinfra", "internal/computeruse",
    "internal/tools",
}

def git(root, *args):
    return subprocess.check_output(
        ["git", "-C", str(root), *args], text=True
    ).strip()

def line_number(text, offset):
    return text.count("\n", 0, offset) + 1

def local(name):
    return any(name == m or name.startswith(m + "/") for m in ROOTS)

packages = {}
for module, root in ROOTS.items():
    actual = git(root, "rev-parse", "HEAD")
    if actual != PINS[module]:
        raise SystemExit(f"pin mismatch: {root}: {actual}")
    if git(root, "status", "--porcelain"):
        raise SystemExit(f"dirty source repository: {root}")
    tracked = git(root, "ls-files", "*.go").splitlines()
    for relative in sorted(tracked):
        path = root / relative
        text = path.read_text()
        directory = pathlib.PurePosixPath(relative).parent.as_posix()
        package = module + ("/" + directory if directory != "." else "")
        imports = []
        for declaration in IMPORT_DECL.finditer(text):
            for value in re.finditer(r'"([^"]+)"', declaration.group()):
                imports.append({
                    "package": value.group(1),
                    "source": f"{path}:{line_number(text, declaration.start() + value.start())}",
                    "local": local(value.group(1)),
                })
        record = {
            "path": str(path),
            "test": path.name.endswith("_test.go"),
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "imports": imports,
            "build_constraints": [
                {"source": f"{path}:{i}", "text": value}
                for i, value in enumerate(text.splitlines(), 1)
                if value.startswith("//go:build ")
                or value.startswith("// +build ")
            ],
            "embed_declarations": [
                {"source": f"{path}:{i}", "patterns": value[11:].strip()}
                for i, value in enumerate(text.splitlines(), 1)
                if value.startswith("//go:embed ")
            ],
        }
        if module == PLATFORM and not record["test"]:
            record["content"] = text
        packages.setdefault(package, []).append(record)

def closure(include_tests):
    seen, queue = set(), collections.deque(SEEDS)
    while queue:
        name = queue.popleft()
        if name in seen:
            continue
        if name not in packages:
            raise SystemExit(f"unresolved local import: {name}")
        seen.add(name)
        for source in packages[name]:
            if source["test"] and not include_tests:
                continue
            queue.extend(
                edge["package"] for edge in source["imports"]
                if edge["local"]
            )
    return seen

production = closure(False)
with_tests = closure(True)
routes = json.loads(pathlib.Path("scripts/platform/package_routes.json").read_text())
records, paths = [], []
for name in sorted(production):
    source_files = sorted(packages[name], key=lambda x: x["path"])
    if name.startswith(PLATFORM + "/"):
        relative = name[len(PLATFORM) + 1:]
        role = (
            "control-plane" if relative in CONTROL_PLANE
            else "worker" if relative in WORKER
            else "shared"
        )
    else:
        relative, role = name[len(SDK) + 1:], "shared-sdk"
    paths.extend(f["path"] for f in source_files if not f["test"])
    records.append({
        "package": name,
        "role": role,
        **routes[name],
        "acceptance_id": "PLAT-PKG-" + hashlib.sha256(name.encode()).hexdigest()[:16].upper(),
        "acceptance_contract": "Preserve worker-reachable observable behavior and wire/storage ABI; run associated Go regressions and Rust replay equivalents. Control-plane internals are retained, not ported.",
        "implementation_status": "not_implemented",
        "verification_status": "not_run",
        "source_test_files": [f["path"] for f in source_files if f["test"]],
        "status": "source-inventoried; Rust verification pending",
        "files": source_files,
    })

result = {
    "format": "platform-local-closure/v1",
    "pins": PINS,
    "seeds": SEEDS,
    "method": "tracked Go import scan; union of all build constraints",
    "limitations": [
        "Package closure is not a reachable-symbol call graph.",
        "Regex import scan is specific to these pins, not a general Go parser; verify.py cross-checks each closure file against Go AST imports.",
        "Build constraints are recorded, not evaluated.",
        "Embed declarations are recorded; patterns are not expanded.",
        "Package role is a boundary label, not a mandate to port every file.",
    ],
    "counts": {
        "packages": len(production),
        "platform_packages": sum(
            p.startswith(PLATFORM + "/") for p in production
        ),
        "sdk_packages": sum(p.startswith(SDK + "/") for p in production),
        "production_files": len(paths),
        "test_files": sum(
            f["test"] for p in production for f in packages[p]
        ),
    },
    "production_path_list_sha256": hashlib.sha256(
        ("\n".join(sorted(paths)) + "\n").encode()
    ).hexdigest(),
    "test_import_added_packages": sorted(with_tests - production),
    "packages": records,
}
print(json.dumps(result, indent=2, sort_keys=True))
