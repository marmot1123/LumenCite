"""Offline guardrails for fixed replay; no application or credentials required."""
import io
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch
import replay_sequence


class FakeShim:
    def __init__(self, failure=False):
        self.calls = []
        self.failure = failure
        self.process = type('Process', (), {'stdin': io.BytesIO()})()

    def call(self, request):
        self.calls.append(request)
        if request['method'] == 'tools/call':
            result = {'isError': self.failure, 'content': [{'type': 'text', 'text': '{"count":0,"results":[]}'}]}
        else:
            result = {}
        return {'id': request['id'], 'result': result}, 0.0, 10, 20, time.perf_counter_ns()

    def close(self):
        pass


class ReplayTests(unittest.TestCase):
    def test_write_request_is_rejected_before_starting_shim(self):
        with patch.object(replay_sequence, 'Shim') as shim:
            with self.assertRaises(ValueError):
                replay_sequence.replay([{'tool': 'create_entry'}], Path('unused-output'), {})
            shim.assert_not_called()

    def test_sequence_order_is_repeated_and_incomplete_results_stop(self):
        cases = [{'id': str(i), 'tool': 'search_entries', 'arguments': {'query': str(i)}} for i in range(2)]
        for failure in [False, True]:
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temp:
                fake = FakeShim(failure)
                output = Path(temp)/'run'
                with patch.object(replay_sequence, 'Shim', return_value=fake), patch('builtins.print'):
                    if failure:
                        with self.assertRaises(RuntimeError):
                            replay_sequence.replay(cases, output, {}, repeats=2, warmup=0)
                    else:
                        replay_sequence.replay(cases, output, {}, repeats=2, warmup=0)
                calls = [r['params']['arguments']['query'] for r in fake.calls if r['method'] == 'tools/call']
                self.assertEqual(calls, ['0'] if failure else ['0', '1', '0', '1'])
                rows = [json.loads(line) for line in (output/'events.jsonl').read_text().splitlines()]
                parents = [r for r in rows if r['parent_span_id'] is None]
                self.assertEqual(parents[0]['status'], 'incomplete' if failure else 'ok')
                self.assertIsNone(parents[0]['final_answer_ms'])


if __name__ == '__main__':
    unittest.main()
