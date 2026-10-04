//! Local HTTP/HTTPS proxy on 127.0.0.1:8080 (or the next free port up to 8099).
//!
//! Every request is inspected. Requests with findings are either rejected
//! automatically (per the user's rules) or held until the user approves or
//! rejects them in the UI. HTTPS is inspected by terminating TLS with a leaf
//! certificate signed by the per-machine Friction CA (see ca.rs).

use crate::audit::{self, AuditEntry};
use crate::ca::Ca;
use crate::config::{self, Config};
use crate::inspect::{self, InterceptPayload, Threat};
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE, HOST};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response};
use hyper_util::rt::TokioIo;
use rustls_pki_types::ServerName;
use serde::Serialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::oneshot;

pub type BoxErr = Box<dyn std::error::Error + Send + Sync>;
pub type ProxyBody = BoxBody<Bytes, hyper::Error>;

pub const DEFAULT_PORT: u16 = 8080;
const LAST_PORT: u16 = 8099;
static PORT: AtomicU16 = AtomicU16::new(DEFAULT_PORT);

/// The port the proxy is listening on.
pub fn port() -> u16 {
    PORT.load(Ordering::Relaxed)
}

pub fn proxy_addr() -> String {
    format!("127.0.0.1:{}", port())
}
pub const DECISION_TIMEOUT_SECS: u64 = 120;
/// Host used by the in-app demo request. `.invalid` can never resolve, and
/// the proxy answers for it itself, so a demo never leaves the machine.
pub const DEMO_HOST: &str = "demo.friction.invalid";

#[derive(Default)]
pub struct Stats {
    pub inspected: AtomicU64,
    pub held: AtomicU64,
    pub blocked: AtomicU64,
}

#[derive(Serialize)]
pub struct StatsSnapshot {
    pub inspected: u64,
    pub held: u64,
    pub blocked: u64,
}

impl Stats {
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            inspected: self.inspected.load(Ordering::Relaxed),
            held: self.held.load(Ordering::Relaxed),
            blocked: self.blocked.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Decision {
    Approve,
    Reject,
}

pub struct Pending {
    tx: oneshot::Sender<Decision>,
    pub payload: InterceptPayload,
}

#[derive(Clone, Serialize)]
pub struct ProxyStatus {
    pub kind: String,
    pub addr: String,
    pub message: String,
}

pub struct AppState {
    pub pending: Mutex<HashMap<String, Pending>>,
    pub config: Mutex<Config>,
    pub audit_lock: Mutex<()>,
    pub ca: Option<Ca>,
    pub ca_error: Option<String>,
    pub client_tls: Arc<rustls::ClientConfig>,
    pub proxy_status: Mutex<Option<ProxyStatus>>,
    pub stats: Stats,
    pub flight: crate::flight::Recorder,
    pub ledger: Mutex<crate::meter::Ledger>,
    pub rules: Mutex<crate::policy::Loaded>,
}

impl AppState {
    pub fn new() -> Self {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut client = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client.alpn_protocols = vec![b"http/1.1".to_vec()];

        let (ca, ca_error) = match Ca::load_or_create() {
            Ok(ca) => (Some(ca), None),
            Err(e) => {
                eprintln!("[friction] certificate authority unavailable: {e}");
                (None, Some(e.to_string()))
            }
        };

        Self {
            pending: Mutex::new(HashMap::new()),
            config: Mutex::new(config::load()),
            audit_lock: Mutex::new(()),
            ca,
            ca_error,
            client_tls: Arc::new(client),
            proxy_status: Mutex::new(None),
            stats: Stats::default(),
            flight: crate::flight::Recorder::load(),
            ledger: Mutex::new(crate::meter::Ledger::load()),
            rules: {
                crate::policy::ensure_template();
                Mutex::new(crate::policy::reload(&crate::policy::Loaded::default()))
            },
        }
    }

    /// Settings as the UI should show them: app settings with rules.toml applied.
    /// Lock order everywhere: rules, then config.
    pub fn public_config(&self) -> config::PublicConfig {
        let loaded = self.rules.lock().unwrap();
        let eff = crate::policy::effective(&self.config(), &loaded.file);
        let mut p = eff.public();
        p.file_overrides = loaded.status.overrides.clone();
        p
    }

    pub fn spend_summary(&self) -> crate::meter::SpendSummary {
        let (cap, from_file) = {
            let loaded = self.rules.lock().unwrap();
            match loaded.file.budget.daily_ai_spend {
                Some(c) => (Some(c), true),
                None => (self.config().daily_ai_cap, false),
            }
        };
        self.ledger.lock().unwrap().summary(cap, from_file)
    }

    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn resolve(&self, id: &str, decision: Decision) -> bool {
        let pending = self.pending.lock().unwrap().remove(id);
        match pending {
            Some(p) => p.tx.send(decision).is_ok(),
            None => false,
        }
    }

    pub fn pending_payloads(&self) -> Vec<InterceptPayload> {
        let mut list: Vec<InterceptPayload> =
            self.pending.lock().unwrap().values().map(|p| p.payload.clone()).collect();
        list.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        list
    }
}

#[derive(Clone)]
pub struct Target {
    pub https: bool,
    pub host: String,
    pub port: u16,
    /// Label from the proxy URL's username (http://name@127.0.0.1:8080).
    pub agent: Option<String>,
}

/// Dollar amount with enough precision to be meaningful for sub-cent values.
fn usd(v: f64) -> String {
    if v > 0.0 && v < 0.01 {
        format!("${v:.4}")
    } else {
        format!("${v:.2}")
    }
}

fn b64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in input.trim().bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Agent name from `Proxy-Authorization: Basic base64(name:)`.
fn agent_from_proxy_auth(headers: &hyper::HeaderMap) -> Option<String> {
    let v = headers.get("proxy-authorization")?.to_str().ok()?;
    let encoded = v.strip_prefix("Basic ").or_else(|| v.strip_prefix("basic "))?;
    let decoded = String::from_utf8(b64_decode(encoded)?).ok()?;
    let name = decoded.split(':').next()?.trim();
    let clean: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(40)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

impl Target {
    fn authority(&self) -> String {
        let default = if self.https { 443 } else { 80 };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == default { host } else { format!("{host}:{}", self.port) }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn full(b: impl Into<Bytes>) -> ProxyBody {
    Full::new(b.into()).map_err(|never: Infallible| match never {}).boxed()
}

fn text(status: u16, msg: impl Into<String>) -> Response<ProxyBody> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("x-friction", "1")
        .body(full(msg.into()))
        .unwrap()
}

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "proxy-connection",
    "keep-alive",
    "proxy-authorization",
    "proxy-authenticate",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("[friction] notification failed: {e}");
    }
}

fn is_self(host: &str, port: u16) -> bool {
    port == self::port() && matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn info_page() -> Response<ProxyBody> {
    text(
        200,
        &format!("Friction proxy is running.\n\nPoint your agent's HTTP and HTTPS proxy at http://{}.\n", proxy_addr()),
    )
}

/// Resolves where a plain (non-CONNECT) proxied request should go.
fn plain_target(req: &Request<Incoming>) -> Result<Target, Response<ProxyBody>> {
    let uri = req.uri();
    if let Some(host) = uri.host() {
        let https = uri.scheme_str() == Some("https");
        let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
        let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
        if is_self(&host, port) {
            return Err(info_page());
        }
        return Ok(Target { https, host, port, agent: agent_from_proxy_auth(req.headers()) });
    }
    let host_header = req
        .headers()
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| text(400, "Friction: request has no absolute URI and no Host header"))?;
    let authority: hyper::http::uri::Authority = host_header
        .parse()
        .map_err(|_| text(400, "Friction: invalid Host header"))?;
    let host = authority.host().trim_start_matches('[').trim_end_matches(']').to_string();
    let port = authority.port_u16().unwrap_or(80);
    if is_self(&host, port) {
        return Err(info_page());
    }
    Ok(Target { https: false, host, port, agent: agent_from_proxy_auth(req.headers()) })
}

// ---------------------------------------------------------------------------
// Upstream
// ---------------------------------------------------------------------------

async fn send_over<T>(io: T, req: Request<Full<Bytes>>) -> Result<Response<Incoming>, BoxErr>
where
    T: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("[friction] upstream connection error: {e}");
        }
    });
    Ok(sender.send_request(req).await?)
}

pub async fn send_upstream(
    client_tls: Arc<rustls::ClientConfig>,
    target: &Target,
    req: Request<Full<Bytes>>,
) -> Result<Response<Incoming>, BoxErr> {
    let tcp = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::TcpStream::connect((target.host.as_str(), target.port)),
    )
    .await??;
    if target.https {
        let name = ServerName::try_from(target.host.clone())?;
        let tls = tokio_rustls::TlsConnector::from(client_tls).connect(name, tcp).await?;
        send_over(TokioIo::new(tls), req).await
    } else {
        send_over(TokioIo::new(tcp), req).await
    }
}

// ---------------------------------------------------------------------------
// Recording decisions
// ---------------------------------------------------------------------------

struct RequestRecord<'a> {
    method: &'a str,
    url: &'a str,
    headers: &'a [(String, String)],
    body: &'a str,
}

fn record(
    app: &AppHandle,
    state: &AppState,
    payload: &InterceptPayload,
    approved: bool,
    reason: Option<String>,
    req: &RequestRecord,
) {
    let entry = AuditEntry {
        id: format!("aud_{}", payload.id),
        intercept_id: payload.id.clone(),
        outcome: if approved { "approved" } else { "rejected" }.into(),
        agent: payload.agent.clone(),
        merchant: payload.destination.clone(),
        item: payload.summary.clone(),
        amount: if approved { payload.amount_value.unwrap_or(0.0) } else { 0.0 },
        delta: payload.amount_value.unwrap_or(0.0),
        threats_caught: payload.threats.iter().filter(|t| t.holds()).count(),
        timestamp: chrono::Utc::now().timestamp_millis(),
        automatic: reason.is_some().then_some(true),
        reason,
    };
    if !approved {
        state.stats.blocked.fetch_add(1, Ordering::Relaxed);
    }
    {
        let _guard = state.audit_lock.lock().unwrap();
        if !audit::append(&entry) {
            eprintln!("[friction] failed to write audit log");
        }
    }
    let _ = app.emit("audit:added", &entry);
    let flight_reason = entry.reason.clone();
    state.flight.update(app, &payload.id, |d| {
        d.entry.outcome = if approved { "allowed" } else { "blocked" }.into();
        d.entry.reason = flight_reason;
        if !approved {
            d.entry.status = Some(403);
        }
    });
    if !approved {
        state.flight.persist(&payload.id);
    }

    if state.config().rules.log_payloads_locally {
        let threats = serde_json::to_value(&payload.threats).unwrap_or_default();
        audit::archive_payload(&payload.id, &entry.outcome, req.method, req.url, req.headers, req.body, &threats);
    }
}

/// Cleans up a held request if its future is dropped before a decision,
/// which is what happens when the agent disconnects or gives up waiting.
struct HoldGuard {
    app: AppHandle,
    state: Arc<AppState>,
    payload: InterceptPayload,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
    armed: bool,
}

impl Drop for HoldGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let was_pending = self.state.pending.lock().unwrap().remove(&self.payload.id).is_some();
        if !was_pending {
            return;
        }
        let _ = self.app.emit("intercept:expired", &self.payload.id);
        crate::shell::refresh(&self.app);
        let rec = RequestRecord { method: &self.method, url: &self.url, headers: &self.headers, body: &self.body };
        record(
            &self.app,
            &self.state,
            &self.payload,
            false,
            Some("Agent disconnected before you decided. Nothing was sent.".into()),
            &rec,
        );
    }
}

// ---------------------------------------------------------------------------
// Response tap: streams the body through untouched while keeping its first and
// last `limit` bytes for token accounting and the Activity view.
// ---------------------------------------------------------------------------

pub struct Captured {
    pub head: Vec<u8>,
    pub tail: Vec<u8>,
    pub total: u64,
    pub complete: bool,
}

type TapDone = Box<dyn FnOnce(Captured) + Send + Sync>;

struct TapBody {
    inner: std::pin::Pin<Box<Incoming>>,
    head: Vec<u8>,
    tail: Vec<u8>,
    total: u64,
    limit: usize,
    done: Option<TapDone>,
}

impl TapBody {
    fn new(inner: Incoming, limit: usize, done: TapDone) -> Self {
        TapBody { inner: Box::pin(inner), head: Vec::new(), tail: Vec::new(), total: 0, limit, done: Some(done) }
    }

    fn capture(&mut self, d: &[u8]) {
        self.total += d.len() as u64;
        if self.head.len() < self.limit {
            let take = (self.limit - self.head.len()).min(d.len());
            self.head.extend_from_slice(&d[..take]);
            self.tail.extend_from_slice(&d[take..]);
        } else {
            self.tail.extend_from_slice(d);
        }
        if self.tail.len() > self.limit {
            let excess = self.tail.len() - self.limit;
            self.tail.drain(..excess);
        }
    }

    fn finish(&mut self, complete: bool) {
        if let Some(f) = self.done.take() {
            f(Captured {
                head: std::mem::take(&mut self.head),
                tail: std::mem::take(&mut self.tail),
                total: self.total,
                complete,
            });
        }
    }
}

impl hyper::body::Body for TapBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Bytes>, hyper::Error>>> {
        use std::task::Poll;
        let this = self.get_mut();
        match this.inner.as_mut().poll_frame(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(d) = frame.data_ref() {
                    this.capture(d);
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => {
                this.finish(false);
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                this.finish(true);
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for TapBody {
    fn drop(&mut self) {
        self.finish(false);
    }
}

// ---------------------------------------------------------------------------
// Request handling
// ---------------------------------------------------------------------------

pub async fn handle_request(
    app: AppHandle,
    state: Arc<AppState>,
    target: Option<Target>,
    req: Request<Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    if req.method() == Method::CONNECT {
        return Ok(handle_connect(app, state, req));
    }
    let target = match target {
        Some(t) => t,
        None => match plain_target(&req) {
            Ok(t) => t,
            Err(resp) => return Ok(resp),
        },
    };

    let started = std::time::Instant::now();
    let (mut parts, body) = req.into_parts();
    let body_bytes = match body.collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => return Ok(text(400, format!("Friction: could not read request body: {e}"))),
    };

    let path_q = parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
    let url = format!("{}://{}{}", if target.https { "https" } else { "http" }, target.authority(), path_q);
    let method = parts.method.as_str().to_string();
    let is_demo = target.host == DEMO_HOST;
    state.stats.inspected.fetch_add(1, Ordering::Relaxed);
    if !is_demo && !state.config.lock().unwrap().seen_traffic {
        {
            let mut c = state.config.lock().unwrap();
            c.seen_traffic = true;
            config::save(&c);
        }
        let _ = app.emit("config:changed", state.public_config());
    }

    let mut headers: Vec<(String, String)> = parts
        .headers
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
        .collect();
    if let Some(name) = &target.agent {
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("x-friction-agent")) {
            headers.push(("x-friction-agent".into(), name.clone()));
        }
    }
    let agent_name = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-friction-agent"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "unknown-agent".into());
    let rules_file = state.rules.lock().unwrap().file.clone();
    let cfg = crate::policy::effective(&state.config(), &rules_file);

    let json: Option<serde_json::Value> = serde_json::from_slice(&body_bytes).ok();
    let body_out = body_bytes.clone();
    let raw_body = String::from_utf8_lossy(&body_out).to_string();
    let path_only = path_q.split('?').next().unwrap_or("/").to_string();

    // --- AI API calls: metered; ask for an uncompressed reply so usage can be read
    let llm_req = if is_demo {
        None
    } else {
        crate::spend::parse_request(&target.host, &path_q, &body_out).or_else(|| {
            rules_file
                .is_ai_endpoint(&target.host)
                .then(|| crate::spend::parse_request_as("Custom", &path_q, &body_out))
                .flatten()
        })
    };
    if llm_req.is_some() {
        parts.headers.insert(hyper::header::ACCEPT_ENCODING, hyper::header::HeaderValue::from_static("identity"));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let mut threats: Vec<Threat> = Vec::new();

    // --- Secrets: caught everywhere, masked in everything Friction shows or saves
    let secret_findings = {
        let mut f = crate::secrets::scan(&raw_body, &target.host);
        let mut in_url = crate::secrets::scan(&path_q, &target.host);
        in_url.retain(|u| !f.iter().any(|x| x.raw == u.raw));
        f.append(&mut in_url);
        f
    };
    let mut display_findings = crate::secrets::scan(&raw_body, "");
    display_findings.extend(crate::secrets::scan(&path_q, ""));
    let body_text = crate::secrets::redact(&raw_body, &display_findings);
    let url_shown = crate::secrets::redact(&url, &display_findings);

    // --- Payments: known payment APIs, and optionally checkout pages anywhere
    let payment = if cfg.rules.hold_payments {
        // The demo payment is Stripe-shaped and goes through the Stripe detector.
        let host = if is_demo { "api.stripe.com" } else { target.host.as_str() };
        crate::payments::classify(&method, host, &path_only, &raw_body, json.as_ref(), cfg.rules.hold_checkout_pages)
    } else {
        None
    };
    let payment = payment.map(|mut p| {
        if is_demo {
            p.processor = "Stripe (demo)".into();
        }
        p
    });

    if !secret_findings.is_empty() {
        let what: Vec<String> =
            secret_findings.iter().take(4).map(|f| format!("{} ({})", f.kind, f.masked)).collect();
        let more = secret_findings.len().saturating_sub(4);
        let dest = if is_demo {
            json.as_ref().and_then(|j| j.get("destination")).and_then(|v| v.as_str()).unwrap_or("a paste site").to_string()
        } else {
            target.host.clone()
        };
        threats.push(Threat::new(
            "SECRET_LEAK",
            if secret_findings.len() == 1 { "Secret in request" } else { "Secrets in request" },
            "critical",
            "rules",
            format!(
                "{}{} going to {}.",
                what.join(", "),
                if more > 0 { format!(" and {more} more") } else { String::new() },
                dest
            ),
        ));
    }
    if let Some(p) = &payment {
        let detail = if p.generic {
            format!("{method} to a checkout path on {}.", target.host)
        } else {
            match p.amount_text() {
                Some(a) => format!("{} {} for {a}.", p.processor, p.action.to_lowercase()),
                None => format!("{} {}.", p.processor, p.action.to_lowercase()),
            }
        };
        threats.push(Threat::new("PAYMENT", "Payment", "critical", "rules", detail));
    }

    // --- rules.toml: custom block / hold / allow ------------------------------
    let mut rule_block: Option<String> = None;
    match rules_file.verdict(&agent_name, &method, &target.host, &path_only) {
        Some(crate::rules_file::Verdict::Block(r)) => rule_block = Some(r),
        Some(crate::rules_file::Verdict::Hold(r)) => {
            threats.push(Threat::new("CUSTOM_HOLD", "Held by your rules", "warning", "rules", format!("{r}.")))
        }
        Some(crate::rules_file::Verdict::Allow) => threats.retain(|t| t.code == "SECRET_LEAK"),
        None => {}
    }

    // --- Daily AI spend limits ----------------------------------------------
    let mut cap_block: Option<String> = None;
    if llm_req.is_some() {
        let (global, agent_spent) = {
            let l = state.ledger.lock().unwrap();
            (l.today_total(), l.agent_today(&agent_name))
        };
        if let Some(cap) = cfg.daily_ai_cap {
            if global >= cap {
                cap_block = Some(format!(
                    "Blocked automatically: daily AI spend limit reached ({} of {})",
                    usd(global),
                    usd(cap)
                ));
            }
        }
        if cap_block.is_none() {
            if let Some(cap) = crate::policy::agent_ai_cap(&rules_file, &agent_name) {
                if agent_spent >= cap {
                    cap_block = Some(format!(
                        "Blocked automatically: {agent_name} reached its daily AI limit ({} of {})",
                        usd(agent_spent),
                        usd(cap)
                    ));
                }
            }
        }
    }

    // The JSON view is re-serialized from the original body, so mask each line.
    let mut execution_payload = inspect::payload_lines(&method, &url_shown, &body_text, json.as_ref());
    for line in execution_payload.iter_mut() {
        let masked = crate::secrets::redact(&line.text, &display_findings);
        if masked != line.text || display_findings.iter().any(|f| line.text.contains(&f.masked)) {
            line.flagged = Some(true);
        }
        line.text = masked;
    }

    let mut payload_owned = InterceptPayload {
        id: id.clone(),
        agent: agent_name.clone(),
        destination: payment
            .as_ref()
            .filter(|p| !p.generic)
            .map(|p| p.processor.clone())
            .unwrap_or_else(|| target.host.clone()),
        summary: match &payment {
            Some(p) if !p.generic => p.describe(),
            _ => format!("{method} {}", crate::secrets::redact(&path_only, &display_findings)),
        },
        amount: payment.as_ref().and_then(|p| p.amount_text()),
        amount_value: payment.as_ref().and_then(|p| p.amount),
        timestamp: chrono::Utc::now().timestamp_millis(),
        threats,
        execution_payload,
        method: method.clone(),
        url: url_shown.clone(),
        expires_at: 0,
    };

    let rec = RequestRecord { method: &method, url: &url_shown, headers: &headers, body: &body_text };
    let payload = &mut payload_owned;
    let has = |prefix: &str| payload.threats.iter().any(|t| t.code.starts_with(prefix));

    let auto_reason: Option<String> = if let Some(r) = &rule_block {
        Some(format!("Blocked by your rules: {r}"))
    } else if let Some(c) = &cap_block {
        Some(c.clone())
    } else if cfg.rules.block_secret_leaks && has("SECRET_LEAK") {
        Some("Blocked automatically: secret in request".into())
    } else {
        None
    };

    // --- Flight recorder: every request gets an entry -----------------------
    let bodies_saved = cfg.rules.log_payloads_locally;
    let req_len = body_out.len();
    state.flight.start(
        &app,
        crate::flight::FlightDetail {
            entry: crate::flight::FlightEntry {
                id: id.clone(),
                ts: chrono::Utc::now().timestamp_millis(),
                agent: agent_name.clone(),
                method: method.clone(),
                host: target.host.clone(),
                path: crate::secrets::redact(&path_q, &display_findings),
                status: None,
                outcome: if auto_reason.is_some() {
                    "blocked".into()
                } else if payload.threats.iter().any(|t| t.holds()) {
                    "held".into()
                } else {
                    "passed".into()
                },
                reason: None,
                duration_ms: None,
                req_bytes: req_len as u64,
                resp_bytes: 0,
                threats: payload.threats.iter().filter(|t| t.severity != "info").map(|t| t.label.clone()).collect(),
                llm: llm_req.as_ref().map(|r| crate::flight::FlightLlm {
                    provider: r.provider.to_string(),
                    model: r.model.clone(),
                    ..Default::default()
                }),
            },
            url: url_shown.clone(),
            request_headers: headers
                .iter()
                .map(|(k, v)| (k.clone(), crate::audit::redact_header(k, v)))
                .collect(),
            response_headers: Vec::new(),
            request_body: bodies_saved.then(|| crate::flight::clip(&body_text)),
            response_body: None,
            bodies_saved,
        },
    );

    if let Some(reason) = auto_reason {
        record(&app, &state, payload, false, Some(reason.clone()), &rec);
        notify(
            &app,
            &format!("Blocked {}", payload.agent),
            &format!("{}: {}. {}.", payload.destination, payload.summary, reason),
        );
        return Ok(text(403, format!("Friction: {reason}.")));
    }

    // --- Hold for the user's decision ---------------------------------------
    if payload.threats.iter().any(|t| t.holds()) {
        let (tx, rx) = oneshot::channel();
        payload.expires_at = chrono::Utc::now().timestamp_millis() + (DECISION_TIMEOUT_SECS as i64) * 1000;
        state.pending.lock().unwrap().insert(id.clone(), Pending { tx, payload: payload.clone() });
        state.stats.held.fetch_add(1, Ordering::Relaxed);
        let mut guard = HoldGuard {
            app: app.clone(),
            state: state.clone(),
            payload: payload.clone(),
            method: method.clone(),
            url: url_shown.clone(),
            headers: headers.clone(),
            body: body_text.clone(),
            armed: true,
        };

        if let Err(e) = app.emit("intercept:detected", &*payload) {
            eprintln!("[friction] failed to emit intercept: {e}");
            state.pending.lock().unwrap().remove(&id);
            return Ok(text(502, "Friction: failed to notify the UI; request blocked."));
        }
        crate::shell::refresh(&app);
        let worst = payload
            .threats
            .iter()
            .find(|t| t.severity == "critical")
            .or_else(|| payload.threats.first())
            .map(|t| t.label.clone())
            .unwrap_or_default();
        let title = match &payload.amount {
            Some(a) => format!("{} wants to pay {a}", payload.agent),
            None => format!("{} needs your OK", payload.agent),
        };
        notify(&app, &title, &format!("{} · {}. Open Friction to allow or block.", payload.destination, worst));

        let decision = tokio::time::timeout(Duration::from_secs(DECISION_TIMEOUT_SECS), rx).await;
        guard.armed = false;
        state.pending.lock().unwrap().remove(&id);
        crate::shell::refresh(&app);
        match decision {
            Ok(Ok(Decision::Approve)) => {
                record(&app, &state, payload, true, None, &rec);
            }
            Ok(Ok(Decision::Reject)) => {
                record(&app, &state, payload, false, None, &rec);
                return Ok(text(403, "Friction: transaction was rejected by the user."));
            }
            Ok(Err(_)) | Err(_) => {
                state.pending.lock().unwrap().remove(&id);
                let _ = app.emit("intercept:expired", &id);
                let reason = format!("Blocked: no decision within {} minutes", DECISION_TIMEOUT_SECS / 60);
                record(&app, &state, payload, false, Some(reason), &rec);
                return Ok(text(403, "Friction: transaction automatically terminated for safety (no decision)."));
            }
        }
    }

    // --- Forward ------------------------------------------------------------
    if is_demo {
        state.flight.update(&app, &id, |d| {
            d.entry.status = Some(200);
            d.entry.duration_ms = Some(started.elapsed().as_millis() as u64);
        });
        state.flight.persist(&id);
        return Ok(text(200, "Friction demo: allowed. Nothing was sent anywhere."));
    }
    let mut builder = Request::builder().method(parts.method.clone()).uri(path_q.as_str());
    for (k, v) in parts.headers.iter() {
        let name = k.as_str();
        if HOP_BY_HOP.contains(&name) || name == "content-length" {
            continue;
        }
        builder = builder.header(k, v);
    }
    if !parts.headers.contains_key(HOST) {
        builder = builder.header(HOST, target.authority());
    }
    if !body_out.is_empty() || parts.headers.contains_key(CONTENT_LENGTH) {
        builder = builder.header(CONTENT_LENGTH, body_out.len());
    }
    let upstream_req = match builder.body(Full::new(body_out)) {
        Ok(r) => r,
        Err(e) => return Ok(text(400, format!("Friction: could not rebuild request: {e}"))),
    };

    match send_upstream(state.client_tls.clone(), &target, upstream_req).await {
        Ok(resp) => {
            let (mut rp, rb) = resp.into_parts();
            for h in HOP_BY_HOP {
                rp.headers.remove(*h);
            }
            let status = rp.status.as_u16();
            let resp_headers: Vec<(String, String)> = rp
                .headers
                .iter()
                .filter_map(|(k, v)| Some((k.as_str().to_string(), crate::audit::redact_header(k.as_str(), v.to_str().ok()?))))
                .collect();
            let compressed = rp
                .headers
                .get(hyper::header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| !v.eq_ignore_ascii_case("identity"));
            state.flight.update(&app, &id, |d| {
                d.entry.status = Some(status);
                d.response_headers = resp_headers;
            });

            let finish: TapDone = {
                let app = app.clone();
                let state = state.clone();
                let id = id.clone();
                let agent = agent_name.clone();
                let llm_req = llm_req.clone();
                let rules_file = rules_file.clone();
                Box::new(move |cap: Captured| {
                    let duration = started.elapsed().as_millis() as u64;
                    let mut llm_entry = None;
                    if let Some(lr) = &llm_req {
                        let usage = crate::spend::parse_usage(lr.provider, &cap.head, &cap.tail, cap.total, req_len);
                        let model = lr
                            .model
                            .clone()
                            .or_else(|| crate::spend::model_from_response(&cap.head))
                            .unwrap_or_else(|| "unknown".into());
                        let price = crate::policy::price_override(&rules_file, &model)
                            .or_else(|| crate::spend::builtin_price(&model));
                        let cost = crate::spend::cost_usd(&usage, price.as_ref());
                        if (200..300).contains(&status) {
                            state.ledger.lock().unwrap().record(&agent, lr.provider, &model, &usage, cost);
                            let _ = app.emit("spend:changed", state.spend_summary());
                        }
                        llm_entry = Some(crate::flight::FlightLlm {
                            provider: lr.provider.to_string(),
                            model: Some(model),
                            input_tokens: usage.input_tokens + usage.cached_input_tokens,
                            output_tokens: usage.output_tokens,
                            cached_input_tokens: usage.cached_input_tokens,
                            cost_usd: cost,
                            estimated: usage.estimated,
                        });
                    }
                    let body = if !bodies_saved {
                        None
                    } else if compressed {
                        Some(format!("[compressed body, {} bytes]", cap.total))
                    } else {
                        let mut t = String::from_utf8_lossy(&cap.head).to_string();
                        if !cap.tail.is_empty() {
                            t.push_str("\n… [middle omitted] …\n");
                            t.push_str(&String::from_utf8_lossy(&cap.tail));
                        }
                        let found = crate::secrets::scan(&t, "");
                        Some(crate::flight::clip(&crate::secrets::redact(&t, &found)))
                    };
                    let complete = cap.complete;
                    let total = cap.total;
                    state.flight.update(&app, &id, move |d| {
                        d.entry.duration_ms = Some(duration);
                        d.entry.resp_bytes = total;
                        if llm_entry.is_some() {
                            d.entry.llm = llm_entry;
                        }
                        d.response_body = body;
                        if !complete && d.entry.reason.is_none() {
                            d.entry.reason = Some("Response ended early".into());
                        }
                    });
                    state.flight.persist(&id);
                })
            };
            let tapped = TapBody::new(rb, 256 * 1024, finish);
            Ok(Response::from_parts(rp, tapped.boxed()))
        }
        Err(e) => {
            let msg = format!("Friction proxy error reaching {}: {e}", target.authority());
            let reason = msg.clone();
            state.flight.update(&app, &id, |d| {
                d.entry.outcome = "error".into();
                d.entry.reason = Some(reason);
                d.entry.status = Some(502);
                d.entry.duration_ms = Some(started.elapsed().as_millis() as u64);
            });
            state.flight.persist(&id);
            Ok(text(502, msg))
        }
    }
}

/// HTTPS: accept the CONNECT tunnel, terminate TLS with a per-host leaf
/// certificate, and run every decrypted request through handle_request.
fn handle_connect(app: AppHandle, state: Arc<AppState>, req: Request<Incoming>) -> Response<ProxyBody> {
    let Some(authority) = req.uri().authority().cloned() else {
        return text(400, "Friction: CONNECT request without a host");
    };
    let host = authority.host().trim_start_matches('[').trim_end_matches(']').to_string();
    let port = authority.port_u16().unwrap_or(443);
    let agent = agent_from_proxy_auth(req.headers());

    let Some(ca) = state.ca.as_ref() else {
        return text(
            502,
            format!(
                "Friction: HTTPS inspection unavailable ({}); request blocked.",
                state.ca_error.clone().unwrap_or_else(|| "no certificate authority".into())
            ),
        );
    };
    let server_cfg = match ca.server_config_for(&host) {
        Ok(c) => c,
        Err(e) => return text(502, format!("Friction: could not create a certificate for {host}: {e}")),
    };

    tokio::spawn(async move {
        let upgraded = match hyper::upgrade::on(req).await {
            Ok(u) => u,
            Err(e) => {
                eprintln!("[friction] CONNECT upgrade failed: {e}");
                return;
            }
        };
        let acceptor = tokio_rustls::TlsAcceptor::from(server_cfg);
        let tls = match acceptor.accept(TokioIo::new(upgraded)).await {
            Ok(t) => t,
            Err(e) => {
                // Usually: the client does not trust the Friction CA yet.
                eprintln!("[friction] TLS handshake with client failed for {host}: {e}");
                return;
            }
        };
        let target = Target { https: true, host, port, agent };
        let service = service_fn(move |r| handle_request(app.clone(), state.clone(), Some(target.clone()), r));
        if let Err(e) = http1::Builder::new().serve_connection(TokioIo::new(tls), service).await {
            eprintln!("[friction] tunnel error: {e}");
        }
    });

    Response::new(full(Bytes::new()))
}

// ---------------------------------------------------------------------------
// Demo requests, sent through the real proxy pipeline (never leave the machine)
// ---------------------------------------------------------------------------

pub fn send_demo_request(kind: &str) {
    let leak = kind == "leak";
    tauri::async_runtime::spawn(async move {
        let (path, content_type, body) = if leak {
            // AKIAIOSFODNN7EXAMPLE is AWS's own documentation example key. The
            // fake Stripe key is assembled at runtime so repository secret
            // scanners don't mistake this source file for a leak.
            let fake_stripe = format!("{}_{}_{}", "sk", "live", "51DemoOnlyNotARealKey0000");
            let json = serde_json::json!({
                "destination": "pastebin.example",
                "title": "build failure, please help",
                "content": format!(
                    "Error: deploy failed\n\n# .env\nDATABASE_URL=postgres://localhost/app\nAWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\nSTRIPE_SECRET_KEY={fake_stripe}\n"
                )
            });
            ("/paste", "application/json", serde_json::to_vec(&json).unwrap_or_default())
        } else {
            // Same shape as a real Stripe PaymentIntent request ($279.00).
            (
                "/v1/payment_intents",
                "application/x-www-form-urlencoded",
                b"amount=27900&currency=usd&description=Wireless+headphones&confirm=true".to_vec(),
            )
        };
        let req = match Request::builder()
            .method(Method::POST)
            .uri(format!("http://{DEMO_HOST}{path}"))
            .header(HOST, DEMO_HOST)
            .header(CONTENT_TYPE, content_type)
            .header(CONTENT_LENGTH, body.len())
            .header("x-friction-agent", "demo-agent")
            .body(Full::new(Bytes::from(body)))
        {
            Ok(r) => r,
            Err(_) => return,
        };
        let Ok(tcp) = tokio::net::TcpStream::connect(proxy_addr()).await else { return };
        let fut = send_over(TokioIo::new(tcp), req);
        let _ = tokio::time::timeout(Duration::from_secs(DECISION_TIMEOUT_SECS + 10), fut).await;
    });
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

fn set_status(app: &AppHandle, state: &AppState, kind: &str, message: String) {
    let status = ProxyStatus { kind: kind.into(), addr: proxy_addr(), message };
    *state.proxy_status.lock().unwrap() = Some(status.clone());
    let _ = app.emit("proxy:status", status);
    crate::shell::refresh(app);
}

pub async fn run(app: AppHandle, state: Arc<AppState>) {
    // 8080 is a common dev-server port, so fall back to the next free one.
    // The port in use is written to ~/.friction/port for the CLI.
    let mut bound = None;
    for p in DEFAULT_PORT..=LAST_PORT {
        let addr = SocketAddr::from(([127, 0, 0, 1], p));
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => {
                bound = Some((l, addr));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => {
                eprintln!("[friction] proxy bind failed: {e}");
                set_status(&app, &state, "bind_failed", e.to_string());
                return;
            }
        }
    }
    let Some((listener, addr)) = bound else {
        set_status(&app, &state, "port_in_use", format!("ports {DEFAULT_PORT}-{LAST_PORT} are all in use"));
        return;
    };
    PORT.store(addr.port(), Ordering::Relaxed);
    let _ = std::fs::write(config::data_dir().join("port"), format!("{}\n", addr.port()));
    set_status(&app, &state, "ok", String::new());
    println!("[friction] proxy listening on http://{addr}");

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[friction] proxy accept failed: {e}");
                continue;
            }
        };
        let app = app.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let service = service_fn(move |req| handle_request(app.clone(), state.clone(), None, req));
            if let Err(err) = http1::Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await
            {
                eprintln!("[friction] proxy connection error: {err}");
            }
        });
    }
}
