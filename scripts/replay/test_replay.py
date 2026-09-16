"""Offline contract/normalization checks, one behavior per test."""
import copy
import json
from pathlib import Path
import unittest
import replay
from normalize import normalize

ROOT = Path(__file__).resolve().parents[2]

def cases():
    return replay.load_cases(ROOT/'fixtures')

class ReplayTests(unittest.TestCase):
    def test_go_reference_outputs_match_python_replay(self):
        for case in cases():
            with self.subTest(case=case['name']):
                replay.assert_equal(replay.evaluate(case['operation'], case['input']), case['expected'])

    def test_reordered_items_are_not_equivalent(self):
        output = copy.deepcopy(cases()[0]['expected'])
        # Platform transcript contains nine deliberately ordered items.
        output['items'].reverse()
        with self.assertRaises(AssertionError):
            replay.assert_equal(output, cases()[0]['expected'])

    def test_changed_call_reference_is_not_equivalent(self):
        original = next(c for c in cases() if c['operation'] == 'child_event')['expected']
        changed = dict(original, ParentCallID='unrelated')
        with self.assertRaises(AssertionError):
            replay.assert_equal(changed, original)

    def test_false_is_not_omission(self):
        with self.assertRaises(AssertionError):
            replay.assert_equal({'end_turn':False}, {})

    def test_false_is_not_zero(self):
        with self.assertRaises(AssertionError):
            replay.assert_equal({'end_turn':False}, {'end_turn':0})

    def test_null_is_not_empty_array(self):
        with self.assertRaises(AssertionError):
            replay.assert_equal(None, [])

    def test_object_key_order_is_ignored(self):
        replay.assert_equal({'b':2,'a':1}, {'a':1,'b':2})

    def test_unknown_operation_is_rejected(self):
        with self.assertRaises(ValueError):
            replay.evaluate('future-operation', {})

    def test_duplicate_object_keys_are_rejected(self):
        with self.assertRaises(ValueError):
            replay.loads('{"id":1,"id":2}')

    def test_nonfinite_number_is_rejected(self):
        with self.assertRaises(ValueError):
            replay.loads('{"cost":NaN}')

    def test_normalization_is_idempotent(self):
        doc = json.loads((ROOT/'fixtures/sdk.json').read_text())
        self.assertEqual(normalize(doc), normalize(normalize(doc)))

    def test_normalization_orders_trimmed_nanoseconds_chronologically(self):
        stamps = ['2026-01-01T00:00:00Z', '2026-01-01T00:00:00.1Z',
                  '2026-01-01T00:00:00.100000001Z', '2026-01-01T00:00:00.1002Z',
                  '2026-01-01T00:00:00.101Z', '2026-01-01T00:00:01Z']
        document = {'schema_version': 1, 'cases': [{'operation': 'state_ready',
                    'input': [{'event_id': f'e{i}', 'type': 'project.initialized',
                               'time': stamp, 'payload': {}} for i, stamp in enumerate(stamps)],
                    'expected': []}]}
        result = normalize(document)['cases'][0]['input']
        self.assertEqual([e['time'] for e in result],
                         [f'2000-01-01T00:00:00.{i:09d}Z' for i in range(1, 7)])

    def test_normalization_preserves_equal_instants_with_different_precision(self):
        document = {'schema_version': 1, 'cases': [{'operation': 'state_ready',
                    'input': [{'event_id': 'e1', 'type': 'project.initialized',
                               'time': '2026-01-01T00:00:00.1Z',
                               'payload': {'at': '2026-01-01T00:00:00.100000000Z'}}],
                    'expected': []}]}
        event = normalize(document)['cases'][0]['input'][0]
        self.assertEqual(event['time'], event['payload']['at'])

    def test_normalization_rejects_unsupported_timestamp_format(self):
        from normalize import timestamp_key
        for stamp in ['2026-01-01T00:00:00+01:00', '2026-01-01T00:00:00.1234567890Z',
                      '2026-02-30T00:00:00Z']:
            with self.subTest(stamp=stamp), self.assertRaises(ValueError):
                timestamp_key(stamp)

    def test_normalization_retains_dependency_relationship(self):
        doc = normalize(json.loads((ROOT/'fixtures/sdk.json').read_text()))
        state = next(c for c in doc['cases'] if c['operation'] == 'state_ready')
        created = [e['payload'] for e in state['input'] if e['type']=='task.created']
        self.assertEqual(created[1]['depends_on'], [created[0]['id']])

    def test_state_sequence_reordering_is_rejected(self):
        state = copy.deepcopy(next(c for c in cases() if c['operation'] == 'state_ready'))
        state['input'].reverse()
        with self.assertRaises(ValueError):
            replay.evaluate(state['operation'],state['input'])

    def test_nonexistent_dependency_is_rejected(self):
        state = copy.deepcopy(next(c for c in cases() if c['operation'] == 'state_ready'))
        state['input'][2]['payload']['depends_on']=['missing']
        with self.assertRaises(ValueError):
            replay.evaluate(state['operation'],state['input'])

if __name__ == '__main__':
    unittest.main()
