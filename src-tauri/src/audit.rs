//! On-disk audit log (~/.friction/audit_log.json) and optional payload archive.

use crate::config::{data_dir, write_private};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: String,
    pub intercept_id: String,
    pub outcome: String,
    pub agent: String,
    pub merchant: String,
    pub item: String,
    pub amount: f64,
    pub delta: f64,
    pub threats_caught: usize,
    pub timestamp: i64,
    /// True when Friction decided on its own (a rule or a timeout).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn audit_path() -> std::path::PathBuf {
    data_dir().join("audit_log.json")
}

pub fn load() -> Vec<AuditEntry> {
    std::fs::read_to_string(audit_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(entries: &[AuditEntry]) -> bool {
    match serde_json::to_vec_pretty(entries) {
        Ok(json) => write_private(&audit_path(), &json).is_ok(),
        Err(_) => false,
    }
}

pub fn append(entry: &AuditEntry) -> bool {
    let mut entries = load();
    entries.retain(|e| e.id != entry.id);
    entries.insert(0, entry.clone());
    save(&entries)
}

const SECRET_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "api-key",
];

/// Header value with credentials hidden.
pub fn redact_header(name: &str, value: &str) -> String {
    if SECRET_HEADERS.contains(&name.to_lowercase().as_str()) {
        "[redacted]".into()
    } else {
        value.to_string()
    }
}

/// Forensic copy of a resolved request. Credential headers are redacted.
pub fn archive_payload(
    id: &str,
    outcome: &str,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: &str,
    threats: &serde_json::Value,
) {
    let dir = data_dir().join("payloads");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let redacted: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| {
            if SECRET_HEADERS.contains(&k.to_lowercase().as_str()) {
                (k.clone(), "[redacted]".into())
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect();
    let doc = serde_json::json!({
        "id": id,
        "outcome": outcome,
        "savedAt": chrono::Utc::now().to_rfc3339(),
        "method": method,
        "url": url,
        "headers": redacted,
        "body": body,
        "threats": threats,
    });
    if let Ok(json) = serde_json::to_vec_pretty(&doc) {
        let safe_id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let _ = write_private(&dir.join(format!("{safe_id}.json")), &json);
    }
}
