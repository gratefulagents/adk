#!/usr/bin/env python3
"""Offline Python reference for the bounded Go-derived migration corpus.
Not a provider emulator or replacement SDK. JSON candidate protocol documented in README.
"""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import sys

TYPES = ['message','tool_call','tool_output','handoff_call','handoff_output','reasoning','tool_approval','compaction']
FIELDS = ['Message','ToolCall','ToolOutput','HandoffCall','HandoffOutput','Reasoning','ToolApproval','Compaction']

def loads(text):
    def pairs(items):
        out = {}
        for key, value in items:
            if key in out:
                raise ValueError(f'duplicate key: {key}')
            out[key] = value
        return out
    def invalid(value):
        raise ValueError(f'nonfinite number: {value}')
    return json.loads(text, object_pairs_hook=pairs, parse_constant=invalid)

def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False, allow_nan=False)

def assert_equal(actual, expected):
    if canonical(actual) != canonical(expected):
        raise AssertionError(f'expected {canonical(expected)}\nactual   {canonical(actual)}')

def snapshot_items(items):
    if not items:
        return None
    output = []
    for item in items:
        index = item['Type']
        if type(index) is not int or index not in range(len(TYPES)):
            raise ValueError('unsupported run item type')
        kind = TYPES[index]
        snap = {'type':kind}
        if item.get('Agent') and item['Agent'].get('Name'):
            snap['agent_name'] = item['Agent']['Name']
        value = deepcopy(item.get(FIELDS[index]))
        if value is not None:
            if kind == 'message':
                for source, target in [('text','message_text'),('phase','message_phase'),('images','message_images')]:
                    if value.get(source):
                        snap[target] = value[source]
            elif kind == 'reasoning':
                if value.get('text'):
                    value['thinking'] = value['text']
                    snap['reasoning_text'] = value['text']
                    snap['thinking_text'] = value['text']
                snap[kind] = value
            else:
                snap[kind] = value
        output.append(snap)
    return output

def response_snapshot(response):
    if response is None:
        return None
    items = snapshot_items(response['Items'])
    out = {'usage':deepcopy(response['Usage']), 'raw_available':response['Raw'] is not None}
    if items:
        out['items'] = items
    if response['EndTurn'] is not None:
        out['end_turn'] = response['EndTurn']
    if response['Raw'] is not None:
        out['raw'] = deepcopy(response['Raw'])
    for item in items or []:
        for source,target in [('message_text','texts'),('reasoning','reasoning'),('reasoning_text','reasoning_texts'),('thinking_text','thinking_texts'),('tool_call','tool_calls')]:
            if source in item:
                out.setdefault(target,[]).append(item[source])
    return out

def child_event(event):
    if not event.get('parent_call_id') or event['type'] not in ('tool_start','tool_end'):
        return None
    end = event['type'] == 'tool_end'
    return dict(ParentCallID=event['parent_call_id'],CallID=event.get('tool_use_id',''),
                AgentName=event.get('agent_name',''),Tool=event.get('tool',''),Phase='end' if end else 'start',
                InputRaw='' if end else event.get('input_raw',''),Output=event.get('output','') if end else '',
                IsError=event.get('is_error',False) if end else False,DurationMS=event.get('tool_duration_ms',0) if end else 0)

def persist_transcript(source):
    out = {key:deepcopy(value) for key,value in source.items() if key != 'items'}
    out['items'] = []
    for item in source['items']:
        kind = item['type']
        if kind not in TYPES:
            raise ValueError('unsupported persisted item')
        row = {'type':kind}
        if 'agent_name' in item:
            row['agent'] = item['agent_name']
        if kind == 'message':
            # Bounded text-only fixture: image stripping is separately covered by Go baseline.
            row['message'] = {'text':item.get('message_text','')}
        elif kind in item:
            row[kind] = deepcopy(item[kind])
            if kind == 'reasoning':
                row[kind].pop('thinking',None)
        out['items'].append(row)
    return out

def state_ready(events):
    tasks = {}
    event_ids = set()
    for seq,event in enumerate(events,1):
        if event['seq'] != seq or event['event_id'] in event_ids:
            raise ValueError('out-of-order or duplicate state event')
        event_ids.add(event['event_id'])
        payload,kind = event['payload'],event['type']
        if kind == 'project.initialized':
            continue
        if kind == 'task.created':
            if payload['id'] in tasks:
                raise ValueError('duplicate task')
            if any(dep not in tasks for dep in payload.get('depends_on',[])):
                raise ValueError('unknown dependency')
            tasks[payload['id']] = deepcopy(payload)
        elif kind == 'task.claimed':
            task = tasks[payload['id']]
            task.update(status='in_progress',assignee=payload['actor'])
        elif kind == 'task.closed':
            if payload['id'] not in tasks:
                raise ValueError('unknown closed task')
            tasks[payload['id']] = deepcopy(payload['task'])
        else:
            raise ValueError(f'unsupported state event {kind}')
    ready = [task for task in tasks.values() if task['status']=='open' and not task.get('assignee')
             and all(tasks[dep]['status']=='closed' for dep in task.get('depends_on',[]))]
    ready.sort(key=lambda task:(task['priority'],task['created_at'],task['id']))
    return [task['id'] for task in ready]

OPERATIONS = dict(snapshot_items=snapshot_items,response_snapshot=response_snapshot,child_event=child_event,
                  persist_transcript=persist_transcript,state_ready=state_ready)

def evaluate(operation, data):
    if operation not in OPERATIONS:
        raise ValueError(f'unsupported operation {operation}')
    return OPERATIONS[operation](data)

def load_cases(directory):
    cases = []
    for name in ('platform.json','sdk.json'):
        doc = loads((directory/name).read_text())
        if doc['schema_version'] != 1:
            raise ValueError('unsupported fixture version')
        cases.extend(doc['cases'])
    if len({case['name'] for case in cases}) != len(cases):
        raise ValueError('duplicate case name')
    return cases

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--fixtures', type=Path, default=Path(__file__).resolve().parents[2]/'fixtures')
    parser.add_argument('--candidate',type=Path,help='compare a foreign implementation name -> output JSON map')
    parser.add_argument('--emit',action='store_true',help='emit reference outputs as candidate JSON')
    parser.add_argument('--evaluate',action='store_true',help='read one {operation,input} from stdin; write output JSON')
    args = parser.parse_args()
    if args.evaluate:
        request = loads(sys.stdin.read())
        print(canonical(evaluate(request['operation'],request['input'])))
        return
    cases = load_cases(args.fixtures)
    results = loads(args.candidate.read_text()) if args.candidate else {c['name']:evaluate(c['operation'],c['input']) for c in cases}
    assert_equal(sorted(results),sorted(c['name'] for c in cases))
    for case in cases:
        try:
            assert_equal(results[case['name']],case['expected'])
        except AssertionError as exc:
            raise AssertionError(f'{case["name"]}: {exc}') from exc
    if args.emit:
        print(json.dumps(results,sort_keys=True,indent=2))
    else:
        print(f'PASS: {len(cases)} cases; exact ordered values and identity relationships retained')

if __name__ == '__main__':
    main()
