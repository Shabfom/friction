//! Secret-leak detection for outgoing requests.
//!
//! Scans request bodies and URLs (not headers: an Authorization header
//! carrying a key to its own API is normal) for credentials. A provider key
//! sent to that provider's own domain is allowed; the same key going anywhere
//! else is a leak. Private keys and `.env`-style secret assignments are never
//! expected in outgoing traffic.

use std::sync::OnceLock;

pub struct Pattern {
    pub kind: &'static str,
    re: regex_lite::Regex,
    /// Domains this credential is legitimately sent to (suffix match).
    pub home: &'static [&'static str],
}

pub struct Finding {
    pub kind: &'static str,
    pub masked: String,
    /// Exact matched text, used to mask it in anything Friction displays or saves.
    pub raw: String,
}

/// Bodies larger than this are only scanned up to this many bytes.
const SCAN_LIMIT: usize = 4 * 1024 * 1024;

fn patterns() -> &'static Vec<Pattern> {
    static P: OnceLock<Vec<Pattern>> = OnceLock::new();
    P.get_or_init(|| {
        let defs: &[(&str, &str, &[&str])] = &[
            ("Private key", r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY(?: BLOCK)?-----", &[]),
            ("Anthropic API key", r"\bsk-ant-[A-Za-z0-9_\-]{20,}", &["anthropic.com"]),
            ("OpenAI API key", r"\bsk-(?:proj-|svcacct-|admin-)?[A-Za-z0-9_\-]{32,}", &["openai.com"]),
            ("AWS access key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", &["amazonaws.com", "aws.amazon.com"]),
            ("GitHub token", r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{60,})", &["github.com", "githubusercontent.com"]),
            ("Stripe secret key", r"\b(?:sk|rk)_live_[A-Za-z0-9]{20,}", &["stripe.com"]),
            ("Slack token", r"\bxox[abprs]-[A-Za-z0-9\-]{10,}", &["slack.com"]),
            ("Google API key", r"\bAIza[0-9A-Za-z_\-]{35}\b", &["googleapis.com", "google.com"]),
            ("Hugging Face token", r"\bhf_[A-Za-z0-9]{34,}\b", &["huggingface.co"]),
            ("npm token", r"\bnpm_[A-Za-z0-9]{36}\b", &["npmjs.org", "npmjs.com"]),
            (
                "Secret from a .env file",
                r#"(?:^|\n|\\n|")\s*(?:export\s+)?[A-Z][A-Z0-9_]*(?:SECRET|TOKEN|PASSWORD|PASSWD|API_KEY|PRIVATE_KEY|ACCESS_KEY)[A-Z0-9_]*\s*=\s*['"]?[^\s'"\\]{8,}"#,
                &[],
            ),
        ];
        defs.iter()
            .filter_map(|(kind, re, home)| {
                regex_lite::Regex::new(re).ok().map(|re| Pattern { kind, re, home })
            })
            .collect()
    })
}

fn host_matches(host: &str, domains: &[&str]) -> bool {
    let h = host.to_lowercase();
    domains.iter().any(|d| h == *d || h.ends_with(&format!(".{d}")))
}

pub fn mask(secret: &str) -> String {
    let s: Vec<char> = secret.trim().chars().collect();
    if s.len() <= 12 {
        return "••••".into();
    }
    let head: String = s[..6].iter().collect();
    let tail: String = s[s.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// Secrets in `text` that are being sent somewhere they don't belong.
pub fn scan(text: &str, host: &str) -> Vec<Finding> {
    let text = if text.len() > SCAN_LIMIT {
        let mut end = SCAN_LIMIT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    } else {
        text
    };
    let mut out: Vec<Finding> = Vec::new();
    for p in patterns() {
        if !p.home.is_empty() && host_matches(host, p.home) {
            continue;
        }
        for m in p.re.find_iter(text).take(5) {
            let raw = m.as_str().trim_start_matches(['\n', '"', ' ']).trim_start_matches("\\n").to_string();
            // sk-ant- keys also fit the OpenAI shape; they're Anthropic's.
            if p.kind == "OpenAI API key" && raw.starts_with("sk-ant-") {
                continue;
            }
            // An OpenAI-shaped match inside an Anthropic key is the same secret.
            if out.iter().any(|f| f.raw.contains(&raw) || raw.contains(&f.raw)) {
                continue;
            }
            let shown = match raw.find('=') {
                Some(i) if p.kind.starts_with("Secret from") => {
                    format!("{}={}", raw[..i].trim().trim_start_matches("export ").trim(), mask(&raw[i + 1..]))
                }
                _ if p.kind == "Private key" => "-----BEGIN … PRIVATE KEY-----".into(),
                _ => mask(&raw),
            };
            out.push(Finding { kind: p.kind, masked: shown, raw });
        }
    }
    out
}

/// Replaces every found secret in `text` with its masked form.
pub fn redact(text: &str, findings: &[Finding]) -> String {
    let mut s = text.to_string();
    for f in findings {
        if f.kind == "Private key" {
            // Hide the whole key block, not just its header line.
            if let Some(start) = s.find(&f.raw) {
                let end = s[start..]
                    .find("PRIVATE KEY-----")
                    .and_then(|first| {
                        let after = start + first + 16;
                        s[after..].find("PRIVATE KEY-----").map(|e| after + e + 16)
                    })
                    .unwrap_or(start + f.raw.len());
                s.replace_range(start..end, "-----BEGIN … PRIVATE KEY----- [redacted by Friction]");
            }
        } else {
            s = s.replace(&f.raw, &f.masked);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_aws_key_going_to_pastebin() {
        let f = scan(r#"{"paste":"AKIAIOSFODNN7EXAMPLE"}"#, "pastebin.com");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "AWS access key");
        assert!(!f[0].masked.contains("IOSFODNN7"));
    }

    #[test]
    fn allows_key_to_its_own_provider() {
        let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        assert!(scan(key, "api.anthropic.com").is_empty());
        assert_eq!(scan(key, "evil.example").len(), 1);
    }

    #[test]
    fn finds_env_line_in_json_string() {
        let body = r#"{"content":"DB_HOST=x\nSTRIPE_SECRET_KEY=sk_test_abc123456789\n"}"#;
        let f = scan(body, "api.openai.com");
        assert!(f.iter().any(|x| x.kind == "Secret from a .env file"));
    }

    #[test]
    fn ignores_ordinary_text() {
        assert!(scan(r#"{"note":"the password reset page","token_count":12}"#, "x.com").is_empty());
    }

    #[test]
    fn redacts_private_key_block() {
        let body = "a\n-----BEGIN OPENSSH PRIVATE KEY-----\nAAAAB3Nza\n-----END OPENSSH PRIVATE KEY-----\nb";
        let f = scan(body, "x.com");
        let r = redact(body, &f);
        assert!(!r.contains("AAAAB3Nza"));
    }
}
