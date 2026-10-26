# Performance baseline

This harness follows `docs/lumencite-performance-measurement.md`. It measures
existing read paths without changing application behavior, rebuilding indexes,
or enabling paid model calls. Private scenarios/results belong in the ignored
`performance-results/` directory; keep database snapshots outside the repository.

- L1: `performance_lcir` example calls the real public LCIR loader and context
  builder against a read-only SQLite snapshot. Its sequential child intervals
  partition the measured operation; do not add the parent to its children.
- L2: `measure.py` replays a read-only allowlist through the configured stdio shim
  (which uses authenticated loopback HTTP). It records monotonic client roundtrips.
- L3: independent fresh conversations require the actual client/model and display
  observations. Fixed replay is not L3. Use the generated recording sheet and
  scenarios; leave unavailable observations null.

Warm means prior execution, not a claim that all caches hit. First access in a
new L1 process retains OS caches. A new shim is not a cold application server.
No measurements are silently classified as successful question answers.

## Re-run the private baseline

Python 3.11+ (standard library only), Rust stable, and the configured LumenCite
stdio connection are required. The live application must already be running.
The script reads only `mcp_servers.lumencite` from the Codex TOML configuration;
it never writes that configuration or copies credentials to output.

Read `performance-results/LATEST` to locate the private run directory. Its
`environment.json` points to the SQLite online-backup snapshot outside Dropbox.
`cases.json` contains exact MCP arguments and mechanical retrieval checks;
`scenarios.json` contains the ten user prompts. These are intentionally not public
fixtures. Output directories/files must be new, so original results stay intact.

```bash
python3 -m unittest discover -s benchmarks/performance -v
python3 benchmarks/performance/measure.py --cases /absolute/run/cases.json \
  --output /absolute/new-l2-run --warmup 1 --repeats 5 --server-build debug
# Narrow follow-up: five warmups, thirty measured trials per selected case.
python3 benchmarks/performance/measure.py --cases /absolute/run/cases-confirmatory.json \
  --output /absolute/new-l2-confirmatory --warmup 5 --repeats 30 --server-build debug
```

From `src-tauri/`, build the optional example, then run the resulting binary
**after compilation finishes**. Do not compile or run other benchmarks concurrently
with the timed trials. Here `TARGET` means Cargo's actual configured target
location (this workstation uses a directory outside Dropbox):

```text
cargo build --release --example performance_lcir
TARGET/release/examples/performance_lcir SNAPSHOT_DB L1_CASES_JSON NEW_OUTPUT_JSONL 35
```

The first five iterations per case are warmups, followed by thirty measured
iterations. `LUMENCITE_PERF_DETAIL=0` retains the parent timing but suppresses
child-span logging, for paired logging-overhead comparisons. This compares
**detailed versus minimal instrumentation**, not a claim of zero overhead from
timing itself. A missing fixture/context or IO error exits nonzero; preserve the
exit status and partial output instead of reporting those cases as complete.

For first access after a process restart, pass a one-case JSON file and `1` as
the repetition count; repeat in five separate processes. These rows are tagged
`first_case_access`/`warmup` by the executable; the run manifest identifies them
as the separate process-cold experiment. OS caches are not cleared. The pool
is established before the clock origin, so pool initialization/process startup
are excluded from these operation times.

L1 returns the context components before the MCP wrapper (no citation-key and
available-version metadata). Its byte count cannot be substituted for the MCP
response size, and L1 release must not be subtracted from L2 debug to estimate
transport overhead. L2 setup rows time initialize and tools/list separately;
they are excluded from connected-session tool-call statistics.

The timer stops before response decoding, correctness checks and log IO in L2.
Byte counts include the JSON-RPC UTF-8 stdio line/newline, not HTTP headers or
inferred tokens. No response text or request arguments enter the performance log.
Candidate identifiers are retained for rank analysis; text lives only in the
original library, not performance logs.

`summary.csv` reports successful RPC latency separately from mechanical retrieval
checks, failures and timeouts. Missing measurements are null; small samples have
no P95. A timeout terminates the replay to prevent late-response misattribution.
The first exploratory B4 check intentionally remains in the run: its assumption
that the search would return zero results was falsified. This is a fixture
expectation failure, not evidence that the tool failed or that the LLM answered
incorrectly. Do not silently revise historical results.

`--server-build` must describe the observed live server (not merely the shim).
Omit it to record `unknown`; never infer a release server from a release shim.

Reaggregate all saved L1 and L2 runs and check parent/child span containment:

```bash
python3 benchmarks/performance/summarize.py /absolute/run
```

To create a consistent snapshot, use Python's SQLite backup API with a read-only
source connection (`sqlite3.connect(source.as_uri() + '?mode=ro', uri=True)`).
Back up into a new destination outside the repository, close both connections,
and record the destination SHA-256. Do not use a bare copy of a live WAL database.
The snapshot must contain the saved fixture node/version IDs; re-extraction can
change IDs, so an unrelated or newly rebuilt database is not the same workload.

## Fixed search/read sequence replay

For broad questions, save the exact ordered read requests as another private
fixture and use the sequential replay runner:

```bash
python3 benchmarks/performance/replay_sequence.py --cases /absolute/run/sequence-cases.json \
  --output /absolute/run/l2-sequence --warmup 1 --repeats 5
```

The parent `mcp_sequence` interval measures a whole fixed request sequence,
including response decoding and checks between requests. Its children measure
individual MCP roundtrips. Log IO occurs after the parent interval ends. The
parent includes the children: never add the two levels together. There are no
LLM decisions or answer generation, and `first_evidence_ms` / `final_answer_ms`
remain null. An error stops the sequence and preserves incomplete records.

The full-text tool's `N hits\n{...}` response is parsed to collect hit counts and
candidate identifiers. Metadata search uses `id`; block/full-text search uses
`entry_id`. These identifiers are normalized only in measurement metadata.
