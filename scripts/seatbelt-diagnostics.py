#!/usr/bin/env python3
"""Print bounded crash reasons for fixed CI sandbox probes, never memory/argv."""
import json
import pathlib

roots = [pathlib.Path.home() / "Library/Logs/DiagnosticReports",
         pathlib.Path("/Library/Logs/DiagnosticReports")]
for root in roots:
    for path in sorted(root.glob("*.ips")):
        if not path.name.startswith(("env-", "sh-", "sandbox-exec-", "git-", "python3-", "Python-")):
            continue
        try:
            text = path.read_text()
            # Apple IPS files contain a metadata JSON line and then the report.
            decoder = json.JSONDecoder()
            _, end = decoder.raw_decode(text)
            report = json.loads(text[end:])
            selected = {key: report[key] for key in ("termination", "exception", "asi")
                        if key in report}
            print(path.name, json.dumps(selected, ensure_ascii=True)[:8000])
        except (OSError, ValueError) as error:
            print(path.name, type(error).__name__)
