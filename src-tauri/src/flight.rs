//! Flight recorder: a record of every request agents send through Friction,
//! kept in memory for the Activity view and appended to
//! ~/.friction/flight/YYYY-MM-DD.jsonl. Exports to HAR 1.2.

use crate::config::data_dir;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

const MEMORY_LIMIT: usize = 2000;
const RETENTION_DAYS: i64 = 14;
/// Bodies are kept up to this size (per direction) in Activity and on disk.
pub const BODY_LIMIT: usize = 32 * 1024;

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FlightLlm {
    pub provider: String,
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: Option<f64>,
    pub estimated: bool,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FlightEntry {
    pub id: String,
    pub ts: i64,
    pub agent: String,
    pub method: String,
    pub host: String,
    pub path: String,
    pub status: Option<u16>,
    /// passed | allowed | blocked | held | error
    pub outcome: String,
    pub reason: Option<String>,
    pub duration_ms: Option<u64>,
    pub req_bytes: u64,
    pub resp_bytes: u64,
    pub threats: Vec<String>,
    pub llm: Option<FlightLlm>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FlightDetail {
    #[serde(flatten)]
    pub entry: FlightEntry,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub bodies_saved: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FlightQuery {
    pub text: Option<String>,
    pub agent: Option<String>,
    pub llm_only: Option<bool>,
    pub limit: Option<usize>,
}

impl FlightQuery {
    fn matches(&self, e: &FlightEntry) -> bool {
        if let Some(a) = self.agent.as_deref().filter(|a| !a.is_empty()) {
            if e.agent != a {
                return false;
            }
        }
        if self.llm_only.unwrap_or(false) && e.llm.is_none() {
            return false;
        }
        if let Some(t) = self.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            let t = t.to_lowercase();
            let hay = format!("{} {} {} {}", e.agent, e.method, e.host, e.path).to_lowercase();
            if !hay.contains(&t) {
                return false;
            }
        }
        true
    }
}

pub struct Recorder {
    items: Mutex<VecDeque<FlightDetail>>,
}

fn flight_dir() -> std::path::PathBuf {
    let d = data_dir().join("flight");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn clip(text: &str) -> String {
    if text.len() <= BODY_LIMIT {
        return text.to_string();
    }
    let mut end = BODY_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… [truncated, {} bytes total]", &text[..end], text.len())
}

impl Recorder {
    /// Loads recent history from disk and removes files past retention.
    pub fn load() -> Self {
        let dir = flight_dir();
        let cutoff = (chrono::Local::now() - chrono::Duration::days(RETENTION_DAYS))
            .format("%Y-%m-%d.jsonl")
            .to_string();
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        files.retain(|p| p.extension().is_some_and(|x| x == "jsonl"));
        files.sort();
        let mut items: VecDeque<FlightDetail> = VecDeque::new();
        for f in &files {
            let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name < cutoff {
                let _ = std::fs::remove_file(f);
                continue;
            }
        }
        for f in files.iter().rev().take(2).collect::<Vec<_>>().into_iter().rev() {
            if let Ok(text) = std::fs::read_to_string(f) {
                for line in text.lines() {
                    if let Ok(d) = serde_json::from_str::<FlightDetail>(line) {
                        items.push_front(d);
                        if items.len() > MEMORY_LIMIT {
                            items.pop_back();
                        }
                    }
                }
            }
        }
        Recorder { items: Mutex::new(items) }
    }

    pub fn start(&self, app: &AppHandle, detail: FlightDetail) {
        let entry = detail.entry.clone();
        {
            let mut items = self.items.lock().unwrap();
            items.push_front(detail);
            while items.len() > MEMORY_LIMIT {
                items.pop_back();
            }
        }
        let _ = app.emit("flight:added", entry);
    }

    pub fn update(&self, app: &AppHandle, id: &str, f: impl FnOnce(&mut FlightDetail)) {
        let entry = {
            let mut items = self.items.lock().unwrap();
            let Some(d) = items.iter_mut().find(|d| d.entry.id == id) else { return };
            f(d);
            d.entry.clone()
        };
        let _ = app.emit("flight:updated", entry);
    }

    /// Appends the final state of an entry to today's log file.
    pub fn persist(&self, id: &str) {
        let line = {
            let items = self.items.lock().unwrap();
            let Some(d) = items.iter().find(|d| d.entry.id == id) else { return };
            serde_json::to_string(d).unwrap_or_default()
        };
        if line.is_empty() {
            return;
        }
        let path = flight_dir().join(chrono::Local::now().format("%Y-%m-%d.jsonl").to_string());
        let res = (|| -> std::io::Result<()> {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(&path)?;
            writeln!(f, "{line}")
        })();
        if let Err(e) = res {
            eprintln!("[friction] couldn't write flight log: {e}");
        }
    }

    pub fn list(&self, q: &FlightQuery) -> Vec<FlightEntry> {
        let limit = q.limit.unwrap_or(500).min(MEMORY_LIMIT);
        self.items
            .lock()
            .unwrap()
            .iter()
            .filter(|d| q.matches(&d.entry))
            .take(limit)
            .map(|d| d.entry.clone())
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<FlightDetail> {
        self.items.lock().unwrap().iter().find(|d| d.entry.id == id).cloned()
    }

    pub fn clear(&self) -> bool {
        self.items.lock().unwrap().clear();
        if let Ok(rd) = std::fs::read_dir(flight_dir()) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        true
    }

    /// Writes the matching entries as a HAR 1.2 file to `path`.
    pub fn export_har(&self, q: &FlightQuery, path: &std::path::Path) -> Result<(), String> {
        let items: Vec<FlightDetail> = {
            let all = self.items.lock().unwrap();
            let mut v: Vec<FlightDetail> = all
                .iter()
                .filter(|d| q.matches(&d.entry))
                .take(q.limit.unwrap_or(MEMORY_LIMIT))
                .cloned()
                .collect();
            v.reverse(); // chronological
            v
        };
        if items.is_empty() {
            return Err("nothing to export".into());
        }
        let hdrs = |h: &Vec<(String, String)>| -> Vec<serde_json::Value> {
            h.iter()
                .map(|(n, v)| serde_json::json!({ "name": n, "value": crate::audit::redact_header(n, v) }))
                .collect()
        };
        let ctype = |h: &Vec<(String, String)>| -> String {
            h.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        let entries: Vec<serde_json::Value> = items
            .iter()
            .map(|d| {
                let e = &d.entry;
                let started = chrono::DateTime::from_timestamp_millis(e.ts)
                    .unwrap_or_default()
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let time = e.duration_ms.unwrap_or(0) as f64;
                let mut request = serde_json::json!({
                    "method": e.method,
                    "url": d.url,
                    "httpVersion": "HTTP/1.1",
                    "headers": hdrs(&d.request_headers),
                    "queryString": [],
                    "cookies": [],
                    "headersSize": -1,
                    "bodySize": e.req_bytes,
                });
                if let Some(b) = &d.request_body {
                    request["postData"] = serde_json::json!({ "mimeType": ctype(&d.request_headers), "text": b });
                }
                let mut content = serde_json::json!({ "size": e.resp_bytes, "mimeType": ctype(&d.response_headers) });
                if let Some(b) = &d.response_body {
                    content["text"] = serde_json::Value::String(b.clone());
                }
                let mut comment = format!("agent: {}; outcome: {}", e.agent, e.outcome);
                if let Some(r) = &e.reason {
                    comment.push_str(&format!("; {r}"));
                }
                if let Some(l) = &e.llm {
                    comment.push_str(&format!(
                        "; {} {} in/{} out tokens",
                        l.model.clone().unwrap_or_default(),
                        l.input_tokens,
                        l.output_tokens
                    ));
                }
                serde_json::json!({
                    "startedDateTime": started,
                    "time": time,
                    "request": request,
                    "response": {
                        "status": e.status.unwrap_or(0),
                        "statusText": "",
                        "httpVersion": "HTTP/1.1",
                        "headers": hdrs(&d.response_headers),
                        "cookies": [],
                        "content": content,
                        "redirectURL": "",
                        "headersSize": -1,
                        "bodySize": e.resp_bytes,
                    },
                    "cache": {},
                    "timings": { "send": 0, "wait": time, "receive": 0 },
                    "comment": comment,
                })
            })
            .collect();
        let har = serde_json::json!({
            "log": {
                "version": "1.2",
                "creator": { "name": "Friction", "version": env!("CARGO_PKG_VERSION") },
                "entries": entries,
            }
        });
        let json = serde_json::to_vec_pretty(&har).map_err(|e| e.to_string())?;
        // Written in place: the user picked this exact file in a save panel,
        // which is what grants access to it under macOS privacy rules.
        std::fs::write(path, json).map_err(|e| e.to_string())
    }
}
