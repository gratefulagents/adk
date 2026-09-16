# ADK

Rust-native agent development kit and platform-compatible agent work in progress.

The implementation baseline is in **[docs/migration/](docs/migration/README.md)**:
version-pinned Go capability/API ledgers, platform contracts, recorded Go tests,
Go-derived offline fixtures and Rust framework research. No Rust runtime is
implemented by this baseline.

Quick offline verification (Python 3 standard library only):

```sh
python3 scripts/replay/replay.py
python3 -m unittest discover -s scripts/replay -p 'test_*.py' -v
```

See the migration documentation for source pins, regeneration commands, provenance,
upstream licensing and explicitly unverified compatibility obligations.
