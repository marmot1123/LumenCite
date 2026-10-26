import unittest
from trace_to_cases import convert


class TraceConversionTests(unittest.TestCase):
    def rows(self, complete=True):
        return [dict(schema_version=1, session_id='s', event='call_start', request_id=1,
                     method='tools/call', tool_name='get_node_context', safe_arguments={'node_id':42}, replay_complete=complete),
                dict(schema_version=1, session_id='s', event='call_end', request_id=1, status='ok', delivery='sent')]

    def test_safe_arguments_and_private_inputs(self):
        self.assertEqual(convert(self.rows(), [])[0]['arguments'], {'node_id':42})
        with self.assertRaises(ValueError): convert(self.rows(False), [])
        with self.assertRaises(ValueError):
            convert(self.rows(False), [dict(session_id='s', request_id=1, arguments={})])
        inputs=[dict(session_id='s', request_id=1, arguments={'query':'private query'})]
        self.assertEqual(convert(self.rows(False), inputs)[0]['arguments']['query'], 'private query')

    def test_no_write_failed_or_incomplete_replay(self):
        rows=self.rows();rows[0]['tool_name']='update_entry'
        with self.assertRaises(ValueError): convert(rows, [])
        rows=self.rows();rows[1]['status']='timeout'
        with self.assertRaises(ValueError): convert(rows, [])
        with self.assertRaises(ValueError): convert(self.rows()[:1], [])

    def test_conflicting_or_unused_inputs_are_rejected(self):
        with self.assertRaises(ValueError):
            convert(self.rows(), [dict(session_id='s',request_id=1,arguments={'node_id':43})])
        with self.assertRaises(ValueError):
            convert(self.rows(), [dict(session_id='other',request_id=1,arguments={})])


if __name__=='__main__': unittest.main()
