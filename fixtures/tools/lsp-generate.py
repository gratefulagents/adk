"""Run the pinned Go parser without spawning a server or editing the SDK."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[2]
sdk = root / "repos/sdk"
fixtures = root / "fixtures/tools"
commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=sdk, text=True).strip()
assert commit == json.loads((fixtures / "lsp-cases.json").read_text())["sdk_commit"]
with tempfile.TemporaryDirectory() as temp:
    overlay = Path(temp) / "overlay.json"
    overlay.write_text(json.dumps({"Replace": {
        str(sdk / "pkg/agentsdk/tools/lsp/differential_generated_test.go"):
        str(fixtures / "lsp-generate.go")
    }}))
    subprocess.run(["go", "test", "-vet=off", "-overlay", str(overlay),
                    "./pkg/agentsdk/tools/lsp", "-run", "^TestGenerateDifferential$", "-count=1", "-v"],
                   cwd=sdk, env={**os.environ, "LSP_FIXTURES": str(fixtures)}, check=True)
