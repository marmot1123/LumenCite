#!/usr/bin/env python3
"""Aggregate saved baseline events without adding parent and child timings."""
import argparse
import csv
import json
from pathlib import Path
import statistics


def aggregate(directory):
    files = sorted(directory.glob('l1-*.jsonl')) + sorted(directory.glob('l2-*/events.jsonl'))
    groups = {}
    all_rows = []
    for path in files:
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        by_span = {r['span_id']: r for r in rows if 'span_id' in r}
        for row in rows:
            parent_id = row.get('parent_span_id')
            if parent_id:
                parent = by_span[parent_id]
                assert row['start_offset_ms'] >= parent['start_offset_ms'] - 1e-6
                assert row['start_offset_ms'] + row['duration_ms'] <= parent['start_offset_ms'] + parent['duration_ms'] + 1e-6
            combined = {**row, 'measurement_file': str(path.relative_to(directory))}
            all_rows.append(combined)
            cold = path.name.startswith('l1-cold-')
            if row.get('phase') != 'measurement' and not cold:
                continue
            key = (str(path.relative_to(directory)), row['scenario_id'], row['layer'],
                   'process-cold' if cold else row['cache_condition'],
                   row['session_condition'], row['event_or_operation'])
            groups.setdefault(key, []).append(row)
    with (directory/'events.jsonl').open('w') as output:
        for row in all_rows:
            output.write(json.dumps(row, ensure_ascii=False) + '\n')
    with (directory/'summary.csv').open('w', newline='') as output:
        writer = csv.writer(output)
        writer.writerow(['measurement_file','scenario_id','layer','cache_condition','session_condition',
                         'operation','n','ok','failed','timeouts','retrieval_pass','retrieval_fail',
                         'median_ms','min_ms','max_ms','median_response_bytes'])
        for key, rows in groups.items():
            ok = [r for r in rows if r['status'] == 'ok']
            times = [r['duration_ms'] for r in ok]
            sizes = [r['response_bytes'] for r in ok if r.get('response_bytes') is not None]
            writer.writerow([*key,len(rows),len(ok),len(rows)-len(ok),sum(r['status']=='timeout' for r in rows),
                             sum(r.get('retrieval_check') is True for r in rows),sum(r.get('retrieval_check') is False for r in rows),
                             statistics.median(times) if times else None,min(times) if times else None,
                             max(times) if times else None,statistics.median(sizes) if sizes else None])
    return len(all_rows)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    print(f'Aggregated {aggregate(args.directory)} records; span containment checked.')
