"""Offline checks for measurement integrity (no live app, secrets or corpus)."""
import csv
import tempfile
import unittest
from pathlib import Path
from measure import content, summarize, validate

class MeasurementTests(unittest.TestCase):
    def test_fulltext_prefix_is_parsed_without_hiding_invalid_text(self):
        response = {'result': {'content': [{'type': 'text', 'text': '2 hits\n{"count":2,"hits":[{"entry_id":3},{"entry_id":4}]}'}]}}
        self.assertEqual(content(response)['count'], 2)
        response['result']['content'][0]['text'] = '2 hits\nnot JSON'
        self.assertEqual(content(response), '2 hits\nnot JSON')

    def test_failed_and_timeout_trials_are_not_success_latencies(self):
        base={'scenario_id':'A','layer':'L2','cache_condition':'warm','session_condition':'ready','phase':'measurement','response_bytes':10}
        events=[{**base,'status':'ok','duration_ms':5,'retrieval_check':False},
                {**base,'status':'timeout','duration_ms':120000},
                {**base,'status':'error','duration_ms':3}]
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'summary.csv';summarize(events,p)
            with p.open() as f: row=list(csv.DictReader(f))[0]
        self.assertEqual((row['n'],row['ok'],row['failed'],row['timeouts']),('3','1','2','1'))
        self.assertEqual(row['median_ms'],'5')
        self.assertEqual(row['retrieval_fail'],'1')

    def test_missing_observation_is_not_zero_or_success(self):
        self.assertIsNone(validate({},None))
        self.assertFalse(validate({}, {'fields':{'focus.node_id':12}}))
        self.assertTrue(validate({'focus':{'node_id':12}}, {'fields':{'focus.node_id':12}}))

if __name__=='__main__':unittest.main()
