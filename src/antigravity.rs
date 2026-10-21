//! Google Antigravity (agentic IDE).
//!
//! Observed in the shipped app (app.asar + resources/bin/language_server.exe):
//! - Electron spawns `language_server.exe --standalone … --https_server_port 0
//!   --csrf_token <uuid>`; the token is a fresh `crypto.randomUUID()` per launch.
//! - With port 0 the server picks a port and prints
//!   `listening on random port at N for HTTPS`; Electron copies that output
//!   to `%APPDATA%\Antigravity\logs\language_server.log` and loads the UI
//!   from `https://127.0.0.1:N/` (self-signed certificate).
//! - The UI talks Connect RPC to `exa.language_server_pb.LanguageServerService`
//!   with the `x-codeium-csrf-token` header. `GetUserStatus` returns
//!   `userStatus.cascadeModelConfigData.clientModelConfigs[]`, each with a
//!   `label` and `quotaInfo { remainingFraction, resetTime }`;
//!   `GetCascadeModelConfigs` returns `clientModelConfigs` directly.
//!
//! None of this is a public API, so every failure falls back to an
//! "installed" card instead of an error.

use crate::data::{Account, Provider, Row};
use chrono::{DateTime, Local};
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const NAME: &str = "Antigravity";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn installed() -> bool {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let candidates = [
        env("LOCALAPPDATA").map(|p| p.join(r"Programs\Antigravity\Antigravity.exe")),
        env("APPDATA").map(|p| p.join("Antigravity")),
        env("USERPROFILE").map(|p| p.join(r".gemini\antigravity")),
        env("USERPROFILE").map(|p| p.join(".antigravity")),
    ];
    candidates.into_iter().flatten().any(|p| p.exists())
}

fn card(plan: &str, note: &str) -> Provider {
    Provider {
        name: NAME.into(),
        icon: "antigravity".into(),
        accent: Some("#4C8DF6".into()),
        updated_at: None,
        active: 0,
        accounts: vec![Account { name: String::new(), plan: plan.into(), rows: vec![], extras: vec![] }],
        note: (!note.is_empty()).then(|| note.to_string()),
    }
}

pub fn detect() -> Option<Provider> {
    let server = find_server();
    if server.is_none() && !installed() {
        return None;
    }
    let Some((csrf, ports)) = server else {
        return Some(card("installed", "Open **Antigravity** to read its model quotas"));
    };
    let mut candidates = log_ports();
    for p in ports {
        if !candidates.contains(&p) {
            candidates.push(p);
        }
    }
    let Some(status) = candidates.iter().find_map(|&port| user_status(port, &csrf)) else {
        return Some(card("running", "Antigravity is open but its quotas could not be read"));
    };
    Some(parse_status(&status))
}

/// Finds Antigravity's language server: its CSRF token and listening ports.
fn find_server() -> Option<(String, Vec<u16>)> {
    const SCRIPT: &str = r#"
$p = Get-CimInstance Win32_Process -Filter "Name LIKE 'language_server%'" |
     Where-Object { $_.CommandLine -match 'antigravity' } | Select-Object -First 1
if ($p) {
  $ports = @(Get-NetTCPConnection -State Listen -OwningProcess $p.ProcessId -ErrorAction SilentlyContinue |
             Select-Object -ExpandProperty LocalPort -Unique)
  [pscustomobject]@{ cmd = $p.CommandLine; ports = $ports } | ConvertTo-Json -Compress
}"#;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let cmd = v.get("cmd")?.as_str()?;
    let csrf = arg_value(cmd, "--csrf_token")?;
    let ports = match v.get("ports") {
        Some(Value::Array(a)) => a.iter().filter_map(|p| p.as_u64()).map(|p| p as u16).collect(),
        Some(p) => p.as_u64().map(|p| vec![p as u16]).unwrap_or_default(),
        None => vec![],
    };
    Some((csrf, ports))
}

/// Ports announced in the language server log, HTTPS first, newest first.
fn log_ports() -> Vec<u16> {
    let Some(appdata) = std::env::var_os("APPDATA") else { return vec![] };
    let log = PathBuf::from(appdata).join(r"Antigravity\logs\language_server.log");
    let Ok(text) = std::fs::read_to_string(log) else { return vec![] };
    let mut found: Vec<(bool, u16)> = Vec::new();
    for line in text.lines().rev() {
        // "listening on random port at 5387 for HTTPS"
        let Some(i) = line.find(" port at ") else { continue };
        if !line[..i].contains("listening on") {
            continue;
        }
        let rest = &line[i + " port at ".len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let Ok(port) = digits.parse::<u16>() else { continue };
        let https = rest[digits.len()..].trim_start().to_ascii_uppercase().starts_with("FOR HTTPS");
        if !found.iter().any(|(_, p)| *p == port) {
            found.push((https, port));
        }
    }
    found.sort_by_key(|(https, _)| !*https);
    found.into_iter().map(|(_, p)| p).collect()
}

/// Value of `--name value` or `--name=value` in a command line.
fn arg_value(cmd: &str, name: &str) -> Option<String> {
    let mut it = cmd.split_whitespace();
    while let Some(tok) = it.next() {
        if tok == name {
            return it.next().map(|s| s.trim_matches('"').to_string());
        }
        if let Some(v) = tok.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
            return Some(v.trim_matches('"').to_string());
        }
    }
    None
}

fn user_status(port: u16, csrf: &str) -> Option<Value> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .tls_config(Arc::new(localhost_tls()))
        .build();
    let call = |scheme: &str, method: &str, body: Value| -> Option<Value> {
        let url = format!("{scheme}://127.0.0.1:{port}/exa.language_server_pb.LanguageServerService/{method}");
        agent
            .post(&url)
            .set("Content-Type", "application/json")
            .set("Connect-Protocol-Version", "1")
            .set("x-codeium-csrf-token", csrf)
            .send_json(body)
            .ok()?
            .into_json::<Value>()
            .ok()
    };
    let metadata = serde_json::json!({
        "metadata": { "ideName": "antigravity", "extensionName": "antigravity", "locale": "en" }
    });
    for scheme in ["https", "http"] {
        if let Some(v) = call(scheme, "GetUserStatus", metadata.clone()) {
            if v.get("userStatus").is_some() {
                return Some(v);
            }
        }
        if let Some(v) = call(scheme, "GetCascadeModelConfigs", metadata.clone()) {
            if v.get("clientModelConfigs").is_some() {
                return Some(serde_json::json!({ "userStatus": v }));
            }
        }
    }
    None
}

fn parse_status(v: &Value) -> Provider {
    let us = &v["userStatus"];
    let plan = [
        "/planStatus/planInfo/planName",
        "/userTier/name",
        "/planStatus/planInfo/teamsTier",
    ]
    .iter()
    .find_map(|ptr| us.pointer(ptr).and_then(Value::as_str))
    .unwrap_or("Google account");
    let mut p = card(plan, "");

    // Every model config with a `quotaInfo` becomes a row (used = 1 - remaining).
    let mut models = Vec::new();
    collect_quotas(us, &mut models);
    for (label, used, resets_at) in models {
        p.accounts[0].rows.push(Row {
            label,
            short: None,
            percent: used,
            resets_at,
            verb: None,
            detail: None,
        });
    }
    if p.accounts[0].rows.is_empty() {
        p.note = Some("Logged in · no model quota reported".into());
    }
    p.updated_at = Some(Local::now());
    p
}

fn collect_quotas(v: &Value, out: &mut Vec<(String, f32, Option<DateTime<Local>>)>) {
    match v {
        Value::Object(map) => {
            if let (Some(q), Some(label)) = (map.get("quotaInfo"), map.get("label").and_then(Value::as_str)) {
                // A missing remainingFraction means the quota is exhausted.
                let remaining = q.get("remainingFraction").and_then(Value::as_f64).unwrap_or(0.0);
                let resets_at = q
                    .get("resetTime")
                    .and_then(Value::as_str)
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&Local));
                if !out.iter().any(|(l, ..)| l == label) {
                    out.push((label.to_string(), ((1.0 - remaining) * 100.0).clamp(0.0, 100.0) as f32, resets_at));
                }
                return;
            }
            map.values().for_each(|x| collect_quotas(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| collect_quotas(x, out)),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// The language server uses a self-signed certificate on 127.0.0.1 only.

fn localhost_tls() -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptLocalhost))
        .with_no_client_auth()
}

#[derive(Debug)]
struct AcceptLocalhost;

impl rustls::client::danger::ServerCertVerifier for AcceptLocalhost {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}
