"""Schema-v1 normalization: only generated state identity, time and path fields."""
from copy import deepcopy
from datetime import datetime
import re


def timestamp_key(stamp):
    """Order bounded UTC RFC3339Nano values without float/microsecond loss."""
    match = re.fullmatch(r'(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d{1,9}))?Z', stamp)
    if match is None:
        raise ValueError(f'unsupported state timestamp: {stamp}')
    seconds, fraction = match.groups()
    datetime.strptime(seconds, '%Y-%m-%dT%H:%M:%S')  # Validate calendar fields.
    return seconds, int((fraction or '').ljust(9, '0'))

TIME_FIELDS = {'time', 'created_at', 'updated_at', 'closed_at', 'at'}
ID_FIELDS = {'id', 'event_id', 'depends_on', 'blocks', 'task_id', 'depends_on_id'}

def normalize(document):
    doc = deepcopy(document)
    if doc.get('schema_version') != 1:
        raise ValueError('unsupported fixture schema')
    for case in doc['cases']:
        if case['operation'] != 'state_ready':
            continue
        events = case['input']
        identities = {}
        for event in events:
            identities[event['event_id']] = f'event_{len([v for v in identities.values() if v.startswith("event_")])+1}'
            if event['type'] == 'task.created':
                value = event['payload']['id']
                identities[value] = f'task_{len([v for v in identities.values() if v.startswith("task_")])+1}'
            if event['type'] == 'task.closed':
                for comment in event['payload'].get('task', {}).get('comments', []):
                    value = comment['id']
                    identities[value] = f'comment_{len([v for v in identities.values() if v.startswith("comment_")])+1}'
        timestamps = set()
        def collect(value):
            if isinstance(value, dict):
                for key, child in value.items():
                    if key in TIME_FIELDS and isinstance(child, str):
                        timestamps.add(child)
                    else:
                        collect(child)
            elif isinstance(value, list):
                for child in value:
                    collect(child)
        collect(events)
        instants = sorted({timestamp_key(stamp) for stamp in timestamps})
        normalized_times = {instant: f'2000-01-01T00:00:00.{i:09d}Z' for i, instant in enumerate(instants, 1)}
        times = {stamp: normalized_times[timestamp_key(stamp)] for stamp in timestamps}
        def visit(value, key=''):
            if isinstance(value, dict):
                return {k: visit(v, k) for k, v in value.items()}
            if isinstance(value, list):
                return [visit(v, key) for v in value]
            if isinstance(value, str):
                if key in ID_FIELDS or key == 'expected':
                    return identities.get(value, value)
                if key in TIME_FIELDS:
                    return times[value]
                if key == 'state_dir':
                    return '/fixture/state'
            return value
        case['input'] = visit(events)
        case['expected'] = visit(case['expected'], 'expected')
    return doc
