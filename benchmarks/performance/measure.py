#!/usr/bin/env python3
"""Measure fixed read-only MCP requests; never write request/response bodies to logs."""
import argparse
import csv
import hashlib
import json
import os
import re
from pathlib import Path
import selectors
import statistics
import subprocess
import time
import tomllib

READ_TOOLS = {'get_entry', 'get_document_structure', 'get_document_blocks', 'get_node_context',
              'get_fulltext', 'search_entries', 'search_document_nodes', 'fulltext_search',
              'get_node_relations', 'get_symbol_definitions', 'get_figures', 'get_tables',
              'list_collections', 'list_tags', 'resolve_citation_key', 'export_bibtex',
              'find_entries_by_citation_keys'}

class Shim:
    def __init__(self, config):
        self.start = time.perf_counter_ns()
        self.process = subprocess.Popen([config['command'], *config.get('args', [])],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env={**os.environ, **config.get('env', {})}, bufsize=0)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer = b''

    def call(self, request, timeout=120):
        body = json.dumps(request, ensure_ascii=False, separators=(',', ':')).encode() + b'\n'
        start = time.perf_counter_ns()
        self.process.stdin.write(body)
        self.process.stdin.flush()
        deadline = time.monotonic() + timeout
        while b'\n' not in self.buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not self.selector.select(remaining):
                raise TimeoutError('MCP response timeout')
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                raise EOFError('shim exited')
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b'\n', 1)
        end = time.perf_counter_ns()
        response = json.loads(line)
        if response.get('id') != request['id']:
            raise ValueError('MCP response id mismatch')
        return response, (end-start)/1e6, len(body), len(line)+1, start

    def close(self):
        self.selector.close()
        if self.process.poll() is None:
            self.process.terminate()
            try: self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.process.stdin.close()
        self.process.stdout.close()

def content(response):
    result = response.get('result', {})
    text = '\n'.join(x['text'] for x in result.get('content', []) if x.get('type') == 'text')
    try: return json.loads(text)
    except (ValueError, TypeError):
        # fulltext_search returns "N hits\n{...}" rather than a bare JSON string.
        prefix, separator, body = text.partition('\n')
        if separator and re.fullmatch(r"\d+ hits", prefix):
            try:
                return json.loads(body)
            except ValueError:
                pass
        return text

def validate(value, expected):
    """Mechanical retrieval checks, not semantic answer grading."""
    if not expected: return None
    checks = []
    for path, target in expected.get('fields', {}).items():
        actual = value
        for key in path.split('.'):
            actual = actual.get(key) if isinstance(actual, dict) else None
        checks.append(actual == target)
    if 'focus_sha256' in expected:
        focus = value.get('focus', {}).get('text', '') if isinstance(value, dict) else ''
        checks.append(hashlib.sha256(focus.encode()).hexdigest() == expected['focus_sha256'])
    if 'entry_ids_any' in expected:
        rows = value.get('results', []) if isinstance(value, dict) else value
        ids = {x.get('entry_id', x.get('id')) for x in rows if isinstance(x, dict)} if isinstance(rows, list) else set()
        checks.append(bool(ids.intersection(expected['entry_ids_any'])))
    return all(checks) if checks else None

def summarize(events, output):
    groups = {}
    for event in events:
        if event.get('phase') != 'measurement': continue
        key = (event['scenario_id'], event['layer'], event['cache_condition'], event['session_condition'])
        groups.setdefault(key, []).append(event)
    with output.open('w', newline='') as f:
        writer = csv.writer(f)
        writer.writerow(['scenario_id','layer','cache_condition','session_condition','n','ok','failed','timeouts','retrieval_pass','retrieval_fail','median_ms','min_ms','max_ms','median_response_bytes'])
        for key, rows in groups.items():
            ok = [e for e in rows if e['status'] == 'ok']
            values = [e['duration_ms'] for e in ok]
            writer.writerow([*key, len(rows), len(ok), len(rows)-len(ok), sum(e['status']=='timeout' for e in rows), sum(e.get('retrieval_check') is True for e in rows),sum(e.get('retrieval_check') is False for e in rows),
                statistics.median(values) if values else None,min(values) if values else None,max(values) if values else None,
                statistics.median(e['response_bytes'] for e in ok) if ok else None])

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--cases', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--repeats', type=int, default=5)
    parser.add_argument('--server-build', choices=['debug','release','unknown'], default='unknown')
    parser.add_argument('--warmup', type=int, default=1)
    parser.add_argument('--config', type=Path, default=Path.home()/'.codex/config.toml')
    args = parser.parse_args()
    cases = json.loads(args.cases.read_text())
    for case in cases:
        if case['tool'] not in READ_TOOLS: raise ValueError('only read tools allowed')
    config = tomllib.loads(args.config.read_text())['mcp_servers']['lumencite']
    # The existing shim handles keychain authentication; no token is read or saved here.
    args.output.mkdir(parents=True, exist_ok=False)
    events = []
    shim = Shim(config)
    epoch = shim.start
    try:
        for method in ['initialize','tools/list']:
            response, duration, request_bytes, response_bytes, start = shim.call({
                'jsonrpc':'2.0','id':method,'method':method,
                'params': {'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'lumencite-perf','version':'1'}} if method=='initialize' else {}})
            if 'error' in response: raise RuntimeError('MCP initialization failed')
            events.append({'scenario_id':method,'layer':'L2','phase':'setup','duration_ms':duration,
                           'start_offset_ms':(start-epoch)/1e6,'request_bytes':request_bytes,'response_bytes':response_bytes,
                           'tool_count':len(response.get('result',{}).get('tools',[])) if method=='tools/list' else None,
                           'server_info':response.get('result',{}).get('serverInfo')})
        # Protocol notification: the shim intentionally emits no response.
        shim.process.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        shim.process.stdin.flush()
        with (args.output/'events.jsonl').open('x') as log:
            for e in events: log.write(json.dumps(e)+'\n')
            for case in cases:
                for iteration in range(args.warmup+args.repeats):
                    request_id = f"{case['id']}-{iteration}"
                    event = dict.fromkeys(['error_type','response_bytes','result_count','cache_name','cache_hit','db_query_count','lcir_load_count','retrieval_check'])
                    event.update({'run_id':args.output.name,'scenario_id':case['id'],'layer':'L2',
                        'phase':'warmup' if iteration<args.warmup else 'measurement','iteration':iteration,
                        'cache_condition':'warm' if iteration else 'preexisting_unknown',
                        'session_condition':f'initialized_stdio_live_{args.server_build}_server','trace_id':request_id,
                        'span_id':request_id,'parent_span_id':None,'process_id':os.getpid(),
                        'event_or_operation':'mcp_roundtrip','tool_name':case['tool'],
                        'timeout_ms':120000,'retry_index':0,'observation_method':'Python perf_counter_ns; stdin write to complete stdout line',
                        'unavailable_reason':'server spans, UI and model times are unavailable',
                        'request_bytes':None})
                    start = time.perf_counter_ns()
                    try:
                        response, duration, rb, sb, start = shim.call({'jsonrpc':'2.0','id':request_id,
                            'method':'tools/call','params':{'name':case['tool'],'arguments':case['arguments']}})
                        value = content(response)
                        failed = 'error' in response or response.get('result',{}).get('isError',False)
                        event.update(status='error' if failed else 'ok',duration_ms=duration,
                            request_bytes=rb,response_bytes=sb,error_type='rpc_or_tool_error' if failed else None,
                            retrieval_check=validate(value,case.get('expected')) if not failed else False)
                        if isinstance(value,dict):
                            event['result_count']=value.get('count',value.get('returned'))
                            candidates = value.get('results', value.get('hits'))
                            if isinstance(candidates, list):
                                event['candidate_node_ids']=[x.get('node_id') for x in candidates]
                                event['candidate_entry_ids']=[x.get('entry_id', x.get('id')) for x in candidates]
                        event['response_content_sha256']=hashlib.sha256(json.dumps(value,ensure_ascii=False,sort_keys=True).encode()).hexdigest()
                    except Exception as error:
                        event.update(status='timeout' if isinstance(error,TimeoutError) else 'error',
                            duration_ms=(time.perf_counter_ns()-start)/1e6,error_type=type(error).__name__)
                    event['start_offset_ms']=(start-epoch)/1e6
                    log.write(json.dumps(event)+'\n'); log.flush(); events.append(event)
                    print(case['id'],iteration,event['status'],round(event['duration_ms'],2),flush=True)
                    if event['status']=='timeout':
                        # Do not risk attributing a late response to the next request.
                        raise TimeoutError('stopped after timeout; completed rows preserved')
    finally:
        shim.close()
        summarize(events,args.output/'summary.csv')
        (args.output/'transport.json').write_text(json.dumps({'command':config['command'],'args':config.get('args',[]),'token':'OS keychain/env; never recorded','clock':'perf_counter_ns','concurrency':1},indent=2))

if __name__ == '__main__': main()
