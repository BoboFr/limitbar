//! Detects the AI tools installed on this PC, checks whether they are logged
//! in, and reads their usage limits:
//!
//! - **Claude Code**: OAuth token in `~/.claude/.credentials.json`, limits from
//!   `GET /api/oauth/usage` like Claude Code's `/usage`. Per the CLI's own
//!   schema, `limits[]` is the server's authoritative row list
//!   (`kind`, `percent` 0-100, `resets_at`, `scope`); older servers only send
//!   `five_hour` / `seven_day[_*]` objects. `extra_usage` is a monthly credit
//!   spend in minor units, not a window.
//! - **Codex CLI**: `~/.codex/auth.json` for the login. Live limits from
//!   `GET https://chatgpt.com/backend-api/wham/usage` (Bearer token +
//!   `ChatGPT-Account-Id`), as Codex's `/status` does; if that fails, the last
//!   `rate_limits` record Codex wrote in `~/.codex/sessions/**.jsonl`.
//! - **Antigravity**: see `antigravity.rs`.
//!
//! Tokens are only read, never refreshed: refreshing rotates them and would
//! log the real tool out.
//!
//! Tools that are neither installed nor configured are not shown.

use crate::data::{Account, Provider, Row};
use chrono::{DateTime, Local, TimeZone, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| ".".into())
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// True when `exe` (or `exe.cmd` / `exe.exe` / `exe.ps1`) is on the PATH.
fn on_path(exe: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path)
        .any(|dir| ["exe", "cmd", "ps1", "bat"].iter().any(|ext| dir.join(format!("{exe}.{ext}")).is_file()))
}

fn provider(name: &str, icon: &str, plan: &str) -> Provider {
    Provider {
        name: name.into(),
        icon: icon.into(),
        accent: None,
        updated_at: None,
        active: 0,
        accounts: vec![Account { name: String::new(), plan: plan.into(), rows: vec![], extras: vec![] }],
        note: None,
    }
}

fn not_logged_in(name: &str, icon: &str, how: &str) -> Provider {
    Provider { note: Some(format!("Installed but not logged in · run **{how}**")), ..provider(name, icon, "not connected") }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn window_label(minutes: i64) -> (String, String) {
    match minutes {
        300 => ("5-hour".into(), "5h".into()),
        10080 => ("Weekly".into(), "wk".into()),
        m if m % 1440 == 0 => (format!("{}-day", m / 1440), format!("{}d", m / 1440)),
        m if m % 60 == 0 => (format!("{}-hour", m / 60), format!("{}h", m / 60)),
        m => (format!("{m} min"), format!("{m}m")),
    }
}

pub fn detect_all() -> Vec<Provider> {
    [claude(), codex(), crate::antigravity::detect()].into_iter().flatten().collect()
}

// ---------------------------------------------------------------------------
// Claude Code

fn claude() -> Option<Provider> {
    const NAME: &str = "Claude";
    let dir = home().join(".claude");
    let creds = read_json(&dir.join(".credentials.json"));
    let Some(oauth) = creds.as_ref().and_then(|c| c.get("claudeAiOauth")) else {
        return (dir.is_dir() || on_path("claude")).then(|| not_logged_in(NAME, "claude", "claude /login"));
    };

    let sub = oauth.get("subscriptionType").and_then(Value::as_str).unwrap_or("");
    let tier = oauth.get("rateLimitTier").and_then(Value::as_str).unwrap_or("");
    let mut plan = if sub.is_empty() { "Claude".to_string() } else { capitalize(sub) };
    if let Some(mult) = tier.rsplit('_').next().filter(|m| m.ends_with('x') && m[..m.len() - 1].parse::<u32>().is_ok()) {
        plan = format!("{plan} {mult}");
    }
    let mut p = provider(NAME, "claude", &plan);

    let expires = oauth.get("expiresAt").and_then(Value::as_i64).and_then(|ms| Utc.timestamp_millis_opt(ms).single());
    if expires.is_some_and(|t| t <= Utc::now()) {
        p.note = Some("Session expired · open **Claude Code** to refresh it".into());
        return Some(p);
    }
    let Some(token) = oauth.get("accessToken").and_then(Value::as_str) else {
        p.note = Some("No access token in credentials".into());
        return Some(p);
    };

    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let resp = agent
        .get("https://api.anthropic.com/api/oauth/usage")
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", "limitbar")
        .call();
    let body: Value = match resp {
        Ok(r) => match r.into_json() {
            Ok(v) => v,
            Err(e) => {
                p.note = Some(format!("Unexpected response: {e}"));
                return Some(p);
            }
        },
        Err(ureq::Error::Status(401, _)) => {
            p.note = Some("Token rejected · open **Claude Code** to log in again".into());
            return Some(p);
        }
        Err(e) => {
            p.note = Some(format!("Usage unavailable: {}", short_err(&e)));
            return Some(p);
        }
    };

    let acc = &mut p.accounts[0];
    match body.get("limits").and_then(Value::as_array) {
        // Current servers: render the server's rows in its order.
        Some(limits) if !limits.is_empty() => {
            for l in limits {
                let Some(pct) = l.get("percent").and_then(Value::as_f64) else { continue };
                let scope = l
                    .pointer("/scope/model/display_name")
                    .or_else(|| l.pointer("/scope/surface/display_name"))
                    .and_then(Value::as_str);
                let (label, short) = match (l.get("kind").and_then(Value::as_str).unwrap_or(""), scope) {
                    ("session", _) => ("5-hour".to_string(), "5h".to_string()),
                    ("weekly_all", _) => ("Weekly".to_string(), "wk".to_string()),
                    (_, Some(name)) => (format!("{name} wk"), format!("{name} wk")),
                    (kind, None) => (capitalize(&kind.replace('_', " ")), kind.to_string()),
                };
                acc.rows.push(row(&label, &short, pct, parse_time(l.get("resets_at"))));
            }
        }
        // Older servers: one object per window.
        _ => {
            for (key, label, short) in [
                ("five_hour", "5-hour", "5h"),
                ("seven_day", "Weekly", "wk"),
                ("seven_day_overage_included", "Fable wk", "Fable wk"),
                ("seven_day_opus", "Opus wk", "Opus wk"),
                ("seven_day_sonnet", "Sonnet wk", "Sonnet wk"),
            ] {
                let Some(w) = body.get(key).filter(|w| w.is_object()) else { continue };
                let Some(pct) = w.get("utilization").and_then(Value::as_f64) else { continue };
                acc.rows.push(row(label, short, pct.min(100.0), parse_time(w.get("resets_at"))));
            }
        }
    }
    // Extra usage: monthly credits, amounts in minor units (cents for USD).
    if let Some(x) = body.get("extra_usage").filter(|x| x.get("is_enabled").and_then(Value::as_bool) == Some(true)) {
        let used = x.get("used_credits").and_then(Value::as_f64).unwrap_or(0.0) / 100.0;
        let limit = x.get("monthly_limit").and_then(Value::as_f64).map(|l| l / 100.0);
        let cur = match x.get("currency").and_then(Value::as_str).unwrap_or("USD") {
            "USD" => "$".to_string(),
            "EUR" => "€".to_string(),
            c => format!("{c} "),
        };
        p.note = Some(match limit {
            Some(l) => format!("Extra usage **{cur}{used:.2}** / {cur}{l:.0} this month"),
            None => format!("Extra usage **{cur}{used:.2}** this month"),
        });
    }
    let acc = &p.accounts[0];
    if acc.rows.is_empty() && acc.extras.is_empty() && p.note.is_none() {
        p.note = Some("No limits reported for this account".into());
    }
    p.updated_at = Some(Local::now());
    Some(p)
}

fn row(label: &str, short: &str, pct: f64, resets_at: Option<DateTime<Local>>) -> Row {
    Row { label: label.into(), short: Some(short.into()), percent: pct as f32, resets_at, verb: None, detail: None }
}

/// ISO 8601 string or Unix seconds.
fn parse_time(v: Option<&Value>) -> Option<DateTime<Local>> {
    match v? {
        Value::String(s) => DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Local)),
        Value::Number(n) => Local.timestamp_opt(n.as_i64()?, 0).single(),
        _ => None,
    }
}

fn short_err(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(t) => t.kind().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Codex CLI (ChatGPT)

fn codex() -> Option<Provider> {
    const NAME: &str = "ChatGPT · Codex";
    let dir = home().join(".codex");
    let auth = read_json(&dir.join("auth.json"));
    let logged_in = auth.as_ref().is_some_and(|a| {
        a.get("tokens").is_some_and(|t| !t.is_null()) || a.get("OPENAI_API_KEY").is_some_and(|k| k.is_string())
    });
    if !logged_in {
        return (dir.is_dir() || on_path("codex")).then(|| not_logged_in(NAME, "openai", "codex login"));
    }
    if auth.as_ref().is_some_and(|a| a.get("tokens").is_none_or(Value::is_null)) {
        let mut p = provider(NAME, "openai", "API key");
        p.note = Some("API key login · billed per use, no plan limits".into());
        return Some(p);
    }

    let mut p = provider(NAME, "openai", "ChatGPT");
    let tokens = auth.as_ref().and_then(|a| a.get("tokens"));
    if let Some(live) = tokens.and_then(codex_live) {
        return Some(codex_from_live(p, &live));
    }
    let Some((at, limits)) = latest_codex_limits(&dir.join("sessions")) else {
        p.note = Some("Logged in · limits appear after your first Codex session".into());
        return Some(p);
    };
    if let Some(plan) = limits.get("plan_type").and_then(Value::as_str) {
        p.accounts[0].plan = format!("ChatGPT {}", capitalize(plan));
    }
    for key in ["primary", "secondary"] {
        let Some(w) = limits.get(key).filter(|w| !w.is_null()) else { continue };
        let pct = w.get("used_percent").and_then(Value::as_f64).unwrap_or(0.0);
        let (label, short) = window_label(w.get("window_minutes").and_then(Value::as_i64).unwrap_or(0));
        let resets_at = w
            .get("resets_at")
            .and_then(Value::as_i64)
            .and_then(|s| Local.timestamp_opt(s, 0).single())
            .or_else(|| {
                let secs = w.get("resets_in_seconds").and_then(Value::as_i64)?;
                Some(at + chrono::Duration::seconds(secs))
            });
        p.accounts[0].rows.push(row(&label, &short, pct, resets_at));
    }
    if let Some(credits) = limits.get("credits").filter(|c| c.get("has_credits").and_then(Value::as_bool) == Some(true)) {
        let balance = credits.get("balance").map(|b| b.to_string()).unwrap_or_default();
        p.note = Some(format!("Credits balance **{}**", balance.trim_matches('"')));
    }
    p.updated_at = Some(at);
    Some(p)
}

/// `GET /wham/usage` with Codex's own token; None on any failure (expired
/// token, offline…), in which case the session log is used instead.
fn codex_live(tokens: &Value) -> Option<Value> {
    let token = tokens.get("access_token")?.as_str()?;
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let mut req = agent
        .get("https://chatgpt.com/backend-api/wham/usage")
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", "limitbar");
    if let Some(account) = tokens.get("account_id").and_then(Value::as_str) {
        req = req.set("ChatGPT-Account-Id", account);
    }
    let v: Value = req.call().ok()?.into_json().ok()?;
    v.get("rate_limit").is_some().then_some(v)
}

/// `{ plan_type, rate_limit: { primary_window, secondary_window }, credits }`,
/// windows being `{ used_percent, limit_window_seconds, reset_after_seconds, reset_at }`.
fn codex_from_live(mut p: Provider, v: &Value) -> Provider {
    if let Some(plan) = v.get("plan_type").and_then(Value::as_str) {
        p.accounts[0].plan = format!("ChatGPT {}", capitalize(plan));
    }
    let now = Local::now();
    for key in ["primary_window", "secondary_window"] {
        let Some(w) = v.get("rate_limit").and_then(|r| r.get(key)).filter(|w| !w.is_null()) else { continue };
        let pct = w.get("used_percent").and_then(Value::as_f64).unwrap_or(0.0);
        let secs = w.get("limit_window_seconds").and_then(Value::as_i64).unwrap_or(0);
        let (label, short) = window_label(secs / 60);
        let resets_at = parse_time(w.get("reset_at")).or_else(|| {
            let after = w.get("reset_after_seconds").and_then(Value::as_i64)?;
            Some(now + chrono::Duration::seconds(after))
        });
        p.accounts[0].rows.push(row(&label, &short, pct, resets_at));
    }
    if let Some(c) = v.get("credits").filter(|c| c.get("has_credits").and_then(Value::as_bool) == Some(true)) {
        let balance = c.get("balance").map(|b| b.to_string()).unwrap_or_default();
        p.note = Some(format!("Credits balance **{}**", balance.trim_matches('"')));
    }
    if v.pointer("/rate_limit/limit_reached").and_then(Value::as_bool) == Some(true) {
        p.note = Some("**Limit reached**".into());
    }
    p.updated_at = Some(now);
    p
}

/// Scans the newest session files (by path, which is date-ordered) for the
/// last `rate_limits` record. Returns its timestamp and the record.
fn latest_codex_limits(sessions: &Path) -> Option<(DateTime<Local>, Value)> {
    let mut files = Vec::new();
    collect_jsonl(sessions, 0, &mut files);
    files.sort();
    for file in files.iter().rev().take(10) {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        for line in text.lines().rev().filter(|l| l.contains("\"rate_limits\"")) {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            let limits = v.pointer("/payload/rate_limits").or_else(|| v.get("rate_limits"));
            let Some(limits) = limits.filter(|l| l.is_object()) else { continue };
            let at = v
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Local))
                .unwrap_or_else(Local::now);
            return Some((at, limits.clone()));
        }
    }
    None
}

fn collect_jsonl(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() && depth < 4 {
            collect_jsonl(&path, depth + 1, out);
        } else if path.extension().is_some_and(|x| x == "jsonl") {
            out.push(path);
        }
    }
}
