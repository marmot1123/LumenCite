#!/usr/bin/env python3
"""Sequential fixed MCP replay, without LLM decisions or user-visible timing."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import time
import tomllib
from measure import READ_TOOLS, Shim, content, validate


def replay(cases, output, config, repeats=5, warmup=1):
    if any(case['tool'] not in READ_TOOLS for case in cases):
        raise ValueError('only read tools allowed')
    output.mkdir(parents=True, exist_ok=False)
    shim = Shim(config)
    epoch = time.perf_counter_ns()
    try:
        for method in ['initialize', 'tools/list']:
            response, *_ = shim.call({'jsonrpc': '2.0', 'id': method, 'method': method,
                'params': {'protocolVersion': '2024-11-05', 'capabilities': {},
                           'clientInfo': {'name': 'lumencite-fixed-replay', 'version': '1'}} if method == 'initialize' else {}})
            if 'error' in response:
                raise RuntimeError('initialization failed')
        shim.process.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        shim.process.stdin.flush()
        with (output/'events.jsonl').open('x') as log:
            for iteration in range(warmup + repeats):
                started = time.perf_counter_ns()
                trace_id = f'{output.name}-{iteration}'
                parent_id = trace_id + '-sequence'
                records = []
                for step, case in enumerate(cases):
                    request_id = f'{trace_id}-{step}'
                    start = time.perf_counter_ns()
                    record = {'run_id': output.name, 'scenario_id': case['id'], 'layer': 'L2',
                        'iteration': iteration, 'step': step, 'phase': 'warmup' if iteration < warmup else 'measurement',
                        'cache_condition': 'preexisting_unknown' if iteration == 0 else 'warm',
                        'session_condition': 'fixed_sequence_initialized_stdio',
                        'trace_id': trace_id, 'span_id': request_id, 'parent_span_id': parent_id,
                        'event_or_operation': 'mcp_roundtrip', 'tool_name': case['tool'],
                        'process_id': os.getpid(), 'timeout_ms': 120000, 'retry_index': 0,
                        'request_bytes': None, 'response_bytes': None, 'result_count': None,
                        'cache_hit': None, 'db_query_count': None, 'lcir_load_count': None,
                        'retrieval_check': None, 'error_type': None,
                        'observation_method': 'perf_counter_ns, fixed sequential requests without LLM',
                        'unavailable_reason': 'No server spans or user-interface observation'}
                    try:
                        response, duration, rb, sb, start = shim.call({'jsonrpc': '2.0', 'id': request_id,
                            'method': 'tools/call', 'params': {'name': case['tool'], 'arguments': case['arguments']}})
                        value = content(response)
                        failed = 'error' in response or response.get('result', {}).get('isError', False)
                        record.update(duration_ms=duration, request_bytes=rb, response_bytes=sb,
                                      status='error' if failed else 'ok', error_type='rpc_or_tool_error' if failed else None,
                                      retrieval_check=validate(value, case.get('expected')) if not failed else False)
                        record['response_content_sha256'] = hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True).encode()).hexdigest()
                        if isinstance(value, dict):
                            record.update(result_count=value.get('count', value.get('returned')),
                                          source=value.get('source'), truncated=value.get('truncated'), next_block=value.get('next_block'))
                            candidates = value.get('results', value.get('hits', []))
                            record['candidate_entry_ids'] = [x.get('entry_id', x.get('id')) for x in candidates]
                            record['candidate_node_ids'] = [x.get('node_id') for x in candidates]
                            record['returned_node_ids'] = [x.get('node_id') for x in value.get('blocks', [])]
                    except Exception as error:
                        record.update(duration_ms=(time.perf_counter_ns()-start)/1e6,
                                      status='timeout' if isinstance(error, TimeoutError) else 'error', error_type=type(error).__name__)
                    record['start_offset_ms'] = (start-epoch)/1e6
                    records.append(record)
                    if record['status'] != 'ok':
                        break
                ended = time.perf_counter_ns()
                parent = {'run_id': output.name, 'scenario_id': 'C4-sequence', 'layer': 'L2',
                    'iteration': iteration, 'phase': 'warmup' if iteration < warmup else 'measurement',
                    'cache_condition': 'preexisting_unknown' if iteration == 0 else 'warm',
                    'session_condition': 'fixed_sequence_initialized_stdio', 'trace_id': trace_id,
                    'span_id': parent_id, 'parent_span_id': None, 'event_or_operation': 'mcp_sequence',
                    'start_offset_ms': (started-epoch)/1e6, 'duration_ms': (ended-started)/1e6,
                    'tool_interval_sum_ms': sum(r['duration_ms'] for r in records),
                    'tool_call_count': len(records), 'planned_call_count': len(cases),
                    'response_bytes': sum(r['response_bytes'] or 0 for r in records),
                    'status': 'ok' if len(records) == len(cases) and all(r['status'] == 'ok' for r in records) else 'incomplete',
                    'first_evidence_ms': None, 'final_answer_ms': None,
                    'observation_method': 'perf_counter_ns; sequence includes decoding/checking between calls, log IO after timing',
                    'unavailable_reason': 'This is L2 replay, not a model-generated answer or UI measurement'}
                for row in [parent, *records]:
                    log.write(json.dumps(row, ensure_ascii=False)+'\n')
                log.flush()
                print(iteration, parent['status'], round(parent['duration_ms'], 2), parent['response_bytes'], flush=True)
                if parent['status'] != 'ok':
                    raise RuntimeError('stopped incomplete sequence; partial records retained')
    finally:
        shim.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--repeats', type=int, default=5)
    parser.add_argument('--warmup', type=int, default=1)
    args = parser.parse_args()
    config = tomllib.loads((Path.home()/'.codex/config.toml').read_text())['mcp_servers']['lumencite']
    replay(json.loads(args.cases.read_text()), args.output, config, args.repeats, args.warmup)
