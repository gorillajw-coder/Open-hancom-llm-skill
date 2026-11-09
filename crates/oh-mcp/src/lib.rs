//! stdio MCP server. Newline-delimited JSON-RPC 2.0 on a reader/writer pair.
//!
//! Only the high-level document tools exist (DESIGN §3). Cases and drafts live in this
//! process's memory; clients only ever see ids. A bad request, an oversized line or even a
//! handler panic produces an error response and the loop continues.
mod tools;
mod xlsx;

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

pub use tools::{now_stamp, tool_names};

/// Entry instructions; `oh_guide` returns the same text, `oh install` writes it to PROMPT.md.
pub const PROMPT: &str = include_str!("../../../PROMPT.md");
pub const INSTALL_DOC: &str = include_str!("../../../INSTALL.md");

/// One request line may not exceed this (plan + attachments paths are tiny; this is a guard).
pub const MAX_LINE: usize = 4 * 1024 * 1024;
const SUPPORTED: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Debug, Clone)]
pub struct Config {
    pub library: PathBuf,
    pub workdir: PathBuf,
}

pub struct Server {
    pub(crate) cfg: Config,
    pub(crate) state: tools::State,
}

enum Line {
    Eof,
    TooLong,
    Data(Vec<u8>),
}

/// Read one line of at most `max` bytes; longer lines are consumed and reported.
fn read_line<R: BufRead>(r: &mut R, max: usize) -> io::Result<Line> {
    let mut buf = Vec::new();
    let mut too_long = false;
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            return Ok(if buf.is_empty() && !too_long { Line::Eof } else if too_long { Line::TooLong } else { Line::Data(buf) });
        }
        let (take, found) = match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        if !too_long {
            if buf.len() + take > max + 1 {
                too_long = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&chunk[..take]);
            }
        }
        r.consume(take);
        if found {
            return Ok(if too_long { Line::TooLong } else { Line::Data(buf) });
        }
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

impl Server {
    pub fn new(cfg: Config) -> Server {
        let state = tools::State::load(&cfg);
        Server { cfg, state }
    }

    pub fn serve<R: BufRead, W: Write>(&mut self, mut r: R, mut w: W) -> io::Result<()> {
        loop {
            let out = match read_line(&mut r, MAX_LINE)? {
                Line::Eof => return Ok(()),
                Line::TooLong => Some(rpc_error(Value::Null, -32600, "요청이 너무 크다")),
                Line::Data(b) => {
                    if b.iter().all(|c| c.is_ascii_whitespace()) {
                        continue;
                    }
                    match catch_unwind(AssertUnwindSafe(|| self.handle(&b))) {
                        Ok(v) => v,
                        Err(_) => Some(rpc_error(Value::Null, -32603, "내부 오류. 서버는 계속 동작한다")),
                    }
                }
            };
            if let Some(v) = out {
                // A closed pipe ends the server; everything else was already answered.
                writeln!(w, "{v}")?;
                w.flush()?;
            }
        }
    }

    /// One request line to at most one response line.
    pub fn handle(&mut self, line: &[u8]) -> Option<Value> {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return Some(rpc_error(Value::Null, -32700, "JSON 파싱 실패"));
        };
        let Some(obj) = v.as_object() else {
            return Some(rpc_error(Value::Null, -32600, "요청은 JSON 객체여야 한다(배치 미지원)"));
        };
        let id = match obj.get("id") {
            None => None,
            Some(i) if i.is_string() || i.is_number() => Some(i.clone()),
            Some(_) => return Some(rpc_error(Value::Null, -32600, "id는 문자열이나 숫자여야 한다")),
        };
        let reply_err = |code: i64, msg: &str| id.clone().map(|i| rpc_error(i, code, msg));
        if obj.get("jsonrpc") != Some(&json!("2.0")) {
            return reply_err(-32600, "jsonrpc는 \"2.0\"이어야 한다").or_else(|| Some(rpc_error(Value::Null, -32600, "jsonrpc는 \"2.0\"이어야 한다")));
        }
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            // A response-shaped message from the client (result/error) is ignored.
            if obj.contains_key("result") || obj.contains_key("error") {
                return None;
            }
            return reply_err(-32600, "method가 없다").or_else(|| Some(rpc_error(Value::Null, -32600, "method가 없다")));
        };
        let Some(id) = id else {
            return None; // notification (notifications/initialized, cancelled, ...): never answered
        };
        let params = obj.get("params");
        if params.is_some_and(|p| !p.is_object()) {
            return Some(rpc_error(id, -32602, "params는 객체여야 한다"));
        }
        let result = match method {
            "initialize" => {
                let want = params.and_then(|p| p.get("protocolVersion")).and_then(Value::as_str).unwrap_or("");
                let ver = if SUPPORTED.contains(&want) { want } else { SUPPORTED[0] };
                Ok(json!({
                    "protocolVersion": ver,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "open-hancom", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "회사 결재서류(HWPX·XLSX) 작성 도구. 먼저 oh_guide를 호출해 진입 지침을 읽는다."
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools::tool_defs()})),
            "tools/call" => self.call(params, line),
            _ => Err((-32601, "알 수 없는 method".to_string())),
        };
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((c, m)) => rpc_error(id, c, &m),
        })
    }

    fn call(&mut self, params: Option<&Value>, raw_line: &[u8]) -> Result<Value, (i64, String)> {
        let p = params.ok_or((-32602, "params 필요".to_string()))?;
        let name = p.get("name").and_then(Value::as_str).ok_or((-32602, "name 필요".to_string()))?;
        if !tools::tool_names().contains(&name) {
            return Err((-32602, format!("알 수 없는 도구: {name}")));
        }
        let empty = serde_json::Map::new();
        let args = match p.get("arguments") {
            None => &empty,
            Some(Value::Object(m)) => m,
            Some(_) => return Err((-32602, "arguments는 객체여야 한다".to_string())),
        };
        let raw = tools::RawArgs::from_line(raw_line);
        let (ok, body) = match self.call_tool(name, args, &raw) {
            Ok(mut v) => {
                if v.get("ok").is_none() {
                    v["ok"] = json!(true);
                }
                (true, v)
            }
            Err(e) => (false, e.to_json()),
        };
        Ok(json!({"content": [{"type": "text", "text": body.to_string()}], "isError": !ok}))
    }
}
