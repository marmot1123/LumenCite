//! Opt-in, read-only measurement of the existing LCIR read path.
//! Usage: performance_lcir SNAPSHOT_DB CASES_JSON OUTPUT_JSONL REPEATS
use lumencite_lib::{context, ingestion};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    time::Instant,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err("expected SNAPSHOT_DB CASES_JSON OUTPUT_JSONL REPEATS".into());
    }
    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let repeats: usize = args[4].parse()?;
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&args[1])
                .read_only(true)
                .foreign_keys(true),
        )
        .await?;
    let mut out = BufWriter::new(
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&args[3])?,
    );
    let epoch = Instant::now();
    let detailed = std::env::var("LUMENCITE_PERF_DETAIL").as_deref() != Ok("0");
    for case in cases {
        let node_id = case["arguments"]["node_id"]
            .as_i64()
            .ok_or("node_id required")?;
        for iteration in 0..repeats {
            let start = Instant::now();
            let loaded = ingestion::load_node_lcir(&pool, node_id).await?;
            let load_end = Instant::now();
            let (_, _, doc) = loaded.ok_or("fixture node missing")?;
            let bundle =
                context::build_node_context(&doc, node_id, &context::ContextOptions::default())
                    .ok_or("fixture context missing")?;
            let build_end = Instant::now();
            let bytes = serde_json::to_vec(&bundle)?;
            let end = Instant::now();
            let parent = format!(
                "{}-{}-{}",
                std::process::id(),
                case["id"].as_str().unwrap_or("case"),
                iteration
            );
            for (operation, from, to, child) in [
                ("load_context_serialize", start, end, false),
                ("load_node_lcir", start, load_end, true),
                ("build_node_context", load_end, build_end, true),
                ("serialize_context", build_end, end, true),
            ] {
                if child && !detailed {
                    continue;
                }
                writeln!(
                    out,
                    "{}",
                    json!({
                        "run_id": args[3], "scenario_id": case["id"], "layer": "L1",
                        "cache_condition": if iteration == 0 {"first_case_access"} else {"warm"},
                        "session_condition": "read_only_snapshot", "iteration": iteration,
                        "phase": if iteration < 5 {"warmup"} else {"measurement"},
                        "detailed_logging": detailed,
                        "trace_id": parent, "span_id": format!("{parent}-{operation}"),
                        "parent_span_id": if child { Some(format!("{parent}-load_context_serialize")) } else { None },
                        "process_id": std::process::id(), "event_or_operation": operation,
                        "start_offset_ms": from.duration_since(epoch).as_secs_f64()*1000.0,
                        "duration_ms": to.duration_since(from).as_secs_f64()*1000.0,
                        "status": "ok", "error_type": null, "timeout_ms": null, "retry_index": 0,
                        "tool_name": "get_node_context_components", "request_bytes": null,
                        "response_bytes": if child {None} else {Some(bytes.len())},
                        "result_count": null, "loaded_nodes": doc.nodes.len(), "cache_hit": null,
                        "cache_name": null, "db_query_count": null,
                        "lcir_load_count": if operation == "load_node_lcir" {Some(1)} else {None},
                        "observation_method": "Rust Instant; existing public functions",
                        "unavailable_reason": "No per-query or cache instrumentation; excludes MCP wrapper and transport",
                        "build": if cfg!(debug_assertions) {"debug"} else {"release"}
                    })
                )?;
            }
        }
    }
    out.flush()?;
    pool.close().await;
    Ok(())
}
