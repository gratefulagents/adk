#!/usr/bin/env python3
"""Execute pinned Go code to export sanitized offline reference vectors."""
import argparse
import hashlib
import json
import shutil
from pathlib import Path
import subprocess
from normalize import normalize
from run_baseline import ROOT, OUT, environment

PINS = {'sdk':'1dc92b73900fac74dc357a938e4b5eee6392b418', 'gratefulagents':'08e65c970830f05042c251bcbb46ec6a9e3719b9'}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, default=ROOT/'fixtures')
    parser.add_argument('--offline', action='store_true', help='forbid Go dependency resolution over network; requires warm caches')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    work = ROOT/'scripts/replay/.work'
    work.mkdir(exist_ok=True)
    env = environment()
    if args.offline:
        env.update(GOPROXY='off', GOSUMDB='off')
    for repo, pin in PINS.items():
        actual = subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT/'repos'/repo,text=True).strip()
        if actual != pin:
            raise SystemExit(f'{repo}: expected {pin}, got {actual}')
    with (OUT/'sdk-export.log').open('w') as log:
        result = subprocess.run(['go','run','-mod=readonly','../../scripts/replay/export.go'],cwd=ROOT/'repos/sdk',env=env,stdout=subprocess.PIPE,stderr=log,check=True)
    sdk = normalize(json.loads(result.stdout))
    virtual = ROOT/'repos/gratefulagents/cmd/agent/migration_reference_export_test.go'
    overlay = work/'overlay.json'
    overlay.write_text(json.dumps({'Replace':{str(virtual):str(ROOT/'scripts/replay/platform_export_test.go')}}))
    platform_raw = work/'platform.json'
    env['MIGRATION_FIXTURE_OUT'] = str(platform_raw)
    with (OUT/'platform-export.log').open('w') as log:
        subprocess.run(['go','test','-mod=readonly','-p=2','-count=1','-timeout=120s','-overlay='+str(overlay),'-run','^TestExportMigrationReference$','./cmd/agent'],cwd=ROOT/'repos/gratefulagents',env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
    platform = normalize(json.loads(platform_raw.read_text()))
    for name, data in [('sdk',sdk),('platform',platform)]:
        (args.output/(name+'.json')).write_text(json.dumps(data,sort_keys=True,indent=2,ensure_ascii=False)+'\n')
        print(f'exported {name}: {len(data["cases"])} cases')
    sources = {
        'sdk': ['internal/agent/llm_snapshot.go', 'internal/agent/llm_snapshot_test.go',
                'internal/agent/items.go', 'internal/agent/model.go', 'pkg/agentsdk/session_event_stream.go',
                'pkg/agentsdk/session_event_stream_test.go', 'pkg/agentsdk/projectstate/engine.go',
                'pkg/agentsdk/projectstate/filesystem_test.go'],
        'gratefulagents': ['cmd/agent/transcript_snapshot.go', 'cmd/agent/transcript_snapshot_test.go'],
    }
    manifest = {'schema_version':1, 'repositories':{}, 'fixtures':{}}
    (args.output/'licenses').mkdir(exist_ok=True)
    for repo, paths in sources.items():
        license_name = 'SDK-GPL-3.0.txt' if repo == 'sdk' else 'PLATFORM-AGPL-3.0.txt'
        shutil.copyfile(ROOT/'repos'/repo/'LICENSE', args.output/'licenses'/license_name)
        entries = []
        for path in ['LICENSE', *paths]:
            entries.append({'path':path,'sha256':hashlib.sha256((ROOT/'repos'/repo/path).read_bytes()).hexdigest(),
                            'url':f'https://github.com/gratefulagents/{repo}/blob/{PINS[repo]}/{path}'})
        manifest['repositories'][repo] = {'revision':PINS[repo], 'sources':entries, 'license_copy':'licenses/'+license_name}
    for name in ('sdk','platform'):
        manifest['fixtures'][name+'.json'] = hashlib.sha256((args.output/(name+'.json')).read_bytes()).hexdigest()
    (args.output/'manifest.json').write_text(json.dumps(manifest,sort_keys=True,indent=2)+'\n')

if __name__ == '__main__':
    main()
