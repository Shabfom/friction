//! Types shared with the UI and the payload view for held requests.

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadLine {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flagged: Option<bool>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Threat {
    pub code: String,
    pub label: String,
    pub severity: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl Threat {
    pub fn new(code: &str, label: &str, severity: &str, source: &str, detail: String) -> Self {
        Threat {
            code: code.into(),
            label: label.into(),
            severity: severity.into(),
            detail,
            source: Some(source.into()),
        }
    }
    /// critical/warning findings hold the request; info is shown only.
    pub fn holds(&self) -> bool {
        self.severity == "critical" || self.severity == "warning"
    }
}

/// A request waiting for the user's decision. Mirrors `Intercept` in
/// src/lib/friction-data.ts.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterceptPayload {
    pub id: String,
    pub agent: String,
    /// Where it's going (payment processor name or host).
    pub destination: String,
    /// What it does, e.g. "Stripe payment for $20.00" or "POST /v1/upload".
    pub summary: String,
    /// Display amount when a payment amount could be read, e.g. "$20.00".
    pub amount: Option<String>,
    pub amount_value: Option<f64>,
    pub timestamp: i64,
    pub threats: Vec<Threat>,
    pub execution_payload: Vec<PayloadLine>,
    pub method: String,
    pub url: String,
    pub expires_at: i64,
}

/// The request as the user sees it: request line plus a pretty-printed body.
pub fn payload_lines(method: &str, url: &str, body: &str, json: Option<&serde_json::Value>) -> Vec<PayloadLine> {
    let view = match json {
        Some(j) => serde_json::to_string_pretty(j).unwrap_or_else(|_| body.to_string()),
        None if body.contains('=') && !body.contains('\n') && body.len() < 20_000 => {
            // Form-encoded (Stripe style): one field per line.
            body.split('&').collect::<Vec<_>>().join("\n")
        }
        None => body.to_string(),
    };
    let mut lines = vec![PayloadLine { text: format!("{method} {url}"), flagged: None }];
    for line in view.lines().take(400) {
        lines.push(PayloadLine { text: line.to_string(), flagged: None });
    }
    lines
}
