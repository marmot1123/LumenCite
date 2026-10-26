#!/usr/bin/env python3
"""Convert a shim trace to a read-only fixture; never infer omitted arguments."""
import argparse
import json
import os
from pathlib import Path
from measure import READ_TOOLS


def convert(rows, overrides):
    starts = []
    ends = {}
    for row in rows:
        if row.get('schema_version') != 1:
            raise ValueError('unsupported trace schema')
        key = (row['session_id'], row.get('request_id'))
        if row['event'] == 'call_start':
            starts.append((key, row))
        elif row['event'] == 'call_end':
            if key in ends:
                raise ValueError('duplicate call_end')
            ends[key] = row
    replacements = {}
    for row in overrides:
        key = (row['session_id'], row['request_id'])
        if key in replacements or not isinstance(row['arguments'], dict):
            raise ValueError('invalid or duplicate override')
        replacements[key] = row['arguments']
    cases = []
    seen = set()
    for key, row in starts:
        if key in seen:
            raise ValueError('duplicate call_start')
        seen.add(key)
        if row['method'] != 'tools/call':
            continue
        if row.get('tool_name') not in READ_TOOLS:
            raise ValueError('trace contains a write or unsupported tool; replay refused')
        end = ends.get(key)
        if not end or end['status'] != 'ok' or end['delivery'] != 'sent':
            raise ValueError(f'call {key[1]} is unfinished, failed or undelivered; replay refused')
        args = row['safe_arguments']
        if key in replacements:
            extra = replacements.pop(key)
            if not row['replay_complete'] and not (extra.keys() - args.keys()):
                raise ValueError('private arguments were not supplied')
            if any(k in extra and extra[k] != v for k, v in args.items()):
                raise ValueError('override conflicts with recorded arguments')
            args = {**args, **extra}
        elif not row['replay_complete']:
            raise ValueError(f'call {key[1]} needs private arguments; supply --inputs')
        cases.append({'id': f'{key[0]}-{key[1]}', 'tool': row['tool_name'], 'arguments': args})
    if replacements:
        raise ValueError('unused override; check session/request identifiers')
    if not cases:
        raise ValueError('no replayable calls')
    return cases


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('trace', type=Path)
    parser.add_argument('--inputs', type=Path, help='private array of session_id/request_id/arguments objects')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        rows = [json.loads(line) for line in args.trace.read_text().splitlines() if line.strip()]
        overrides = json.loads(args.inputs.read_text()) if args.inputs else []
        cases = convert(rows, overrides)
    except (ValueError, KeyError, TypeError, OSError):
        # Do not echo JSON parser exceptions: they may quote private input text.
        parser.exit(2, 'Trace cannot be replayed: check schema, completed read calls and required private inputs.\n')
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as output:
        json.dump(cases, output, ensure_ascii=False, indent=2)
        output.write('\n')
    print(f'Saved {len(cases)} read-only requests.')


if __name__ == '__main__':
    main()
