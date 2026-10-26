//! Opt-in, metadata-only diagnostics. Never serialize a raw request or response here.
use serde_json::{json, Map, Value};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

pub(super) const READ_TOOLS: &[&str] = &[
    "fulltext_search",
    "search_entries",
    "search_document_nodes",
    "get_entry",
    "get_fulltext",
    "get_document_structure",
    "get_document_blocks",
    "get_node_context",
    "get_node_relations",
    "get_symbol_definitions",
    "get_figures",
    "get_tables",
    "list_collections",
    "list_tags",
    "resolve_citation_key",
    "export_bibtex",
    "find_entries_by_citation_keys",
];
const LIMIT: u64 = 64 * 1024 * 1024;
static SERIAL: AtomicU64 = AtomicU64::new(0);

pub(super) struct Trace {
    file: Option<File>,
    session: String,
    epoch: Instant,
    next: u64,
    bytes: u64,
    known_tools: HashSet<String>,
}

pub(super) struct Ticket {
    id: u64,
    start: Instant,
}

impl Trace {
    pub(super) fn from_env() -> Option<Self> {
        if std::env::var("LUMENCITE_MCP_TRACE").as_deref() != Ok("1") {
            return None;
        }
        let result = std::env::var_os("LUMENCITE_MCP_TRACE_DIR")
            .ok_or_else(|| io::Error::other("missing trace directory"))
            .and_then(|p| Self::open(Path::new(&p)));
        match result {
            Ok(t) => Some(t),
            Err(_) => {
                eprintln!("lumencite-mcp: trace unavailable; forwarding continues");
                None
            }
        }
    }

    pub(super) fn open(dir: &Path) -> io::Result<Self> {
        if !dir.is_absolute() {
            return Err(io::Error::other("absolute directory required"));
        }
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir)?;
        let session = format!(
            "{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_micros(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(dir.join(format!("mcp-trace-{session}.jsonl")))?;
        let known_tools = READ_TOOLS
            .iter()
            .copied()
            .chain([
                "add_tag",
                "update_notes",
                "add_to_collection",
                "create_entry",
                "update_entry",
            ])
            .map(str::to_owned)
            .collect();
        let mut t = Self {
            file: Some(file),
            session,
            epoch: Instant::now(),
            next: 0,
            bytes: 0,
            known_tools,
        };
        t.emit(json!({"event":"session", "started_at_utc":chrono::Utc::now().to_rfc3339(),
            "app_version":env!("CARGO_PKG_VERSION"), "build":if cfg!(debug_assertions) {"debug"} else {"release"},
            "timeout_ms":120000}));
        Ok(t)
    }

    fn emit(&mut self, mut row: Value) {
        if self.file.is_none() {
            return;
        }
        row["schema_version"] = json!(1);
        row["session_id"] = json!(self.session);
        row["process_id"] = json!(std::process::id());
        if row.get("offset_ms").is_none() {
            row["offset_ms"] = json!(self.epoch.elapsed().as_secs_f64() * 1000.0);
        }
        let mut bytes = serde_json::to_vec(&row).expect("trace metadata is JSON");
        bytes.push(b'\n');
        if self.bytes + bytes.len() as u64 > LIMIT {
            self.file = None;
            eprintln!("lumencite-mcp: trace size limit reached; forwarding continues");
            return;
        }
        if self.file.as_mut().unwrap().write_all(&bytes).is_err() {
            self.file = None;
            eprintln!("lumencite-mcp: trace write failed; forwarding continues");
        } else {
            self.bytes += bytes.len() as u64;
        }
    }

    pub(super) fn begin(&mut self, line: &str) -> Option<Ticket> {
        if self.file.is_none() || line.trim().is_empty() {
            return None;
        }
        let start = Instant::now();
        self.next += 1;
        let request: Value = serde_json::from_str(line).unwrap_or(Value::Null);
        let method = request["method"]
            .as_str()
            .filter(|m| {
                matches!(
                    *m,
                    "initialize"
                        | "ping"
                        | "tools/list"
                        | "tools/call"
                        | "notifications/initialized"
                        | "notifications/cancelled"
                )
            })
            .unwrap_or("unknown");
        let name = request["params"]["name"].as_str().unwrap_or("");
        let tool = if method == "tools/call" && self.known_tools.contains(name) {
            Some(name)
        } else {
            None
        };
        let (args, complete) = safe_arguments(tool, &request["params"]["arguments"]);
        self.emit(
            json!({"event":"call_start", "request_id":self.next, "method":method,
            "offset_ms":start.duration_since(self.epoch).as_secs_f64()*1000.0,
            "tool_name":tool, "request_bytes":line.trim().len(), "safe_arguments":args,
            "replay_complete":complete}),
        );
        Some(Ticket {
            id: self.next,
            start,
        })
    }

    pub(super) fn finish(
        &mut self,
        ticket: Ticket,
        outcome: &super::ProxyOutcome,
        forward_start: Instant,
        forward_end: Instant,
        delivery: &str,
    ) {
        let end = Instant::now();
        // Classification examines response shape only. Never copy server-provided strings.
        let status = if outcome.status == "ok" {
            match outcome
                .response
                .as_deref()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
            {
                Some(v) if v.get("error").is_some() => "rpc_error",
                Some(v) if v["result"]["isError"] == true => "tool_error",
                Some(v) if v.get("result").is_some() => "ok",
                _ => "invalid_response",
            }
        } else {
            outcome.status
        };
        self.emit(json!({"event":"call_end", "request_id":ticket.id,
            "offset_ms":end.duration_since(self.epoch).as_secs_f64()*1000.0,
            "duration_ms":end.duration_since(ticket.start).as_secs_f64()*1000.0,
            "forward_start_offset_ms":forward_start.duration_since(self.epoch).as_secs_f64()*1000.0,
            "forward_duration_ms":forward_end.duration_since(forward_start).as_secs_f64()*1000.0,
            "response_bytes":outcome.response_bytes,
            "stdout_bytes":outcome.response.as_ref().map(|s| s.len()+1).unwrap_or(0),
            "status":status, "http_status":outcome.http_status, "delivery":delivery}));
    }
}

fn safe_arguments(tool: Option<&str>, args: &Value) -> (Value, bool) {
    if !tool.is_some_and(|t| READ_TOOLS.contains(&t)) {
        return (json!({}), false);
    }
    let Some(fields) = args.as_object() else {
        return (json!({}), false);
    };
    let mut out = Map::new();
    for (key, value) in fields {
        let safe = match key.as_str() {
            "entry_id"
            | "node_id"
            | "collection_id"
            | "tag_id"
            | "page"
            | "page_start"
            | "block_start"
            | "max_chars"
            | "max_results"
            | "max_before"
            | "max_continuation"
            | "max_continuation_chars"
            | "max_premises"
            | "max_related"
            | "max_relations"
            | "max_symbols"
            | "max_figures"
            | "max_tables" => value.as_i64().is_some(),
            "entry_ids" => value
                .as_array()
                .is_some_and(|v| v.len() <= 64 && v.iter().all(|x| x.as_i64().is_some())),
            "source" => matches!(value.as_str(), Some("tex" | "pdf")),
            _ => false,
        };
        if safe {
            out.insert(key.clone(), value.clone());
        }
    }
    let complete = out.len() == fields.len();
    (Value::Object(out), complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "lumencite-trace-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn logs_are_allowlisted_and_preserve_incomplete_replay_information() {
        let dir = directory();
        let mut trace = Trace::open(&dir).unwrap();
        let secret = "PRIVATE_QUERY_TOKEN_BODY";
        let line = json!({"id":secret,"method":"tools/call","params":{"name":"search_document_nodes",
            "arguments":{"query":secret,"max_results":10, "citation_key":secret, "PRIVATE_KEY":secret}}}).to_string();
        let ticket = trace.begin(&line).unwrap();
        let now = Instant::now();
        trace.finish(
            ticket,
            &super::super::ProxyOutcome {
                response: Some(
                    json!({"result":{"isError":true,"content":[{"text":secret}]}}).to_string(),
                ),
                status: "ok",
                http_status: Some(200),
                response_bytes: Some(100),
            },
            now,
            Instant::now(),
            "sent",
        );
        trace.begin(&json!({"id":secret,"method":secret,"params":{"name":secret}}).to_string());
        drop(trace);
        let file = std::fs::read_dir(&dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains(secret));
        assert!(!text.contains("PRIVATE_KEY"));
        let rows: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows[1]["safe_arguments"], json!({"max_results":10}));
        assert_eq!(rows[1]["replay_complete"], false);
        assert_eq!(rows[2]["status"], "tool_error");
        assert_eq!(rows[1]["request_id"], rows[2]["request_id"]);
        assert_eq!(rows[3]["method"], "unknown");
        assert!(rows[3]["tool_name"].is_null());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn numeric_read_arguments_can_be_replayed_but_text_and_writes_cannot() {
        let args = json!({"node_id":42, "max_related":4});
        assert_eq!(
            safe_arguments(Some("get_node_context"), &args),
            (args, true)
        );
        assert_eq!(
            safe_arguments(Some("update_entry"), &json!({"entry_id":42})),
            (json!({}), false)
        );
        assert!(!safe_arguments(Some("get_entry"), &json!({"entry_id":"secret"})).1);
        assert!(!safe_arguments(Some("get_document_blocks"), &json!({"source":"secret"})).1);
    }

    #[test]
    fn size_limit_stops_logging_without_affecting_caller() {
        let dir = directory();
        let mut trace = Trace::open(&dir).unwrap();
        trace.bytes = LIMIT;
        trace.begin(r#"{"method":"ping","id":1}"#);
        assert!(trace.file.is_none());
        assert!(trace.begin(r#"{"method":"ping","id":2}"#).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_failure_disables_trace_without_panicking() {
        let dir = directory();
        let mut trace = Trace::open(&dir).unwrap();
        let path = std::fs::read_dir(&dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        // A read-only file deterministically rejects writes on every platform.
        trace.file = Some(File::open(path).unwrap());
        trace.begin(r#"{"method":"ping","id":1}"#);
        assert!(trace.file.is_none());
        assert!(trace.begin(r#"{"method":"ping","id":2}"#).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn files_are_distinct_and_relative_paths_are_rejected() {
        assert!(Trace::open(Path::new("relative")).is_err());
        let dir = directory();
        drop(Trace::open(&dir).unwrap());
        drop(Trace::open(&dir).unwrap());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
