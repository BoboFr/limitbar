//! Usage data model. Detected providers are built in `detect.rs`; extra ones
//! can be added by hand in `%APPDATA%\limitbar\usage.json`.

use chrono::{DateTime, Duration, Local, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UsageFile {
    pub providers: Vec<Provider>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Provider {
    pub name: String,
    /// Glyph to draw: "claude", "gemini", "openai", "grok" (anything else draws the initial).
    #[serde(default)]
    pub icon: String,
    /// Accent colour as "#RRGGBB".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Local>>,
    /// Index of the account shown in the card; the others are summarised in the note.
    #[serde(default)]
    pub active: usize,
    pub accounts: Vec<Account>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Account {
    #[serde(default)]
    pub name: String,
    pub plan: String,
    pub rows: Vec<Row>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<Extra>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Row {
    pub label: String,
    /// Short label used in summaries ("5h", "wk").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    pub percent: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<Local>>,
    /// Word before the reset time ("resets" by default, "ends" for credit pools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verb: Option<String>,
    /// Free text shown instead of the reset time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Extra {
    pub label: String,
    pub percent: f32,
}

impl Account {
    pub fn title(&self) -> String {
        if self.name.is_empty() {
            self.plan.clone()
        } else {
            format!("{} ({})", self.name, self.plan)
        }
    }

    pub fn summary(&self) -> String {
        let parts: Vec<String> = self
            .rows
            .iter()
            .map(|r| format!("{} {:.0}%", r.short.as_deref().unwrap_or(&r.label), r.percent))
            .collect();
        format!("{}: {}", self.title(), parts.join(" · "))
    }
}

impl Provider {
    pub fn active_account(&self) -> Option<&Account> {
        self.accounts.get(self.active).or_else(|| self.accounts.first())
    }

    pub fn max_percent(&self) -> f32 {
        self.active_account()
            .map(|a| a.rows.iter().map(|r| r.percent).fold(0.0, f32::max))
            .unwrap_or(0.0)
    }

    pub fn accent_rgb(&self) -> [u8; 3] {
        if let Some(rgb) = self.accent.as_deref().and_then(parse_hex) {
            return rgb;
        }
        match self.icon.as_str() {
            "claude" => [0xD9, 0x77, 0x57],
            "gemini" => [0x8E, 0x7C, 0xF8],
            "openai" => [0x6F, 0xA8, 0xFF],
            _ => [0xC9, 0xCC, 0xD3],
        }
    }
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn data_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| ".".into());
    base.join("limitbar").join("usage.json")
}

/// Loads the optional manual data file (providers the app can't detect).
pub fn load() -> UsageFile {
    match std::fs::read_to_string(data_path()) {
        // Tolerate the UTF-8 BOM that Windows editors and PowerShell add.
        Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .unwrap_or_else(|e| error_file(&e.to_string())),
        Err(_) => UsageFile::default(),
    }
}

/// Collapsed state of the cards, remembered across restarts.
fn ui_state_path() -> PathBuf {
    data_path().with_file_name("ui.json")
}

pub fn load_collapsed() -> std::collections::HashMap<String, bool> {
    std::fs::read_to_string(ui_state_path())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t.trim_start_matches('\u{feff}')).ok())
        .and_then(|v| serde_json::from_value(v.get("collapsed")?.clone()).ok())
        .unwrap_or_default()
}

pub fn save_collapsed(collapsed: &std::collections::HashMap<String, bool>) {
    let path = ui_state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::json!({ "collapsed": collapsed }).to_string());
}

pub fn modified() -> Option<std::time::SystemTime> {
    std::fs::metadata(data_path()).and_then(|m| m.modified()).ok()
}

fn error_file(msg: &str) -> UsageFile {
    UsageFile {
        providers: vec![Provider {
            name: "usage.json".into(),
            icon: String::new(),
            accent: Some("#F0605D".into()),
            updated_at: None,
            active: 0,
            accounts: vec![Account { name: String::new(), plan: "invalid file".into(), rows: vec![], extras: vec![] }],
            note: Some(msg.chars().take(80).collect()),
        }],
    }
}

// ---------------------------------------------------------------------------
// Formatting

/// "21:10", "tomorrow 19:26", "Sat 23:00" or "04 Nov".
pub fn fmt_when(t: DateTime<Local>, now: DateTime<Local>) -> String {
    if t <= now {
        return "now".into();
    }
    let hm = t.format("%H:%M");
    let (day, today) = (t.date_naive(), now.date_naive());
    if day == today {
        hm.to_string()
    } else if Some(day) == today.succ_opt() {
        format!("tomorrow {hm}")
    } else if t - now < Duration::days(6) {
        format!("{} {hm}", t.format("%a"))
    } else {
        t.format("%d %b").to_string()
    }
}

/// "just now", "2m ago" / "2 min ago", "3h ago", "4d ago".
pub fn fmt_ago(t: DateTime<Local>, now: DateTime<Local>, long: bool) -> String {
    let mins = (now - t).num_minutes().max(0);
    match mins {
        0 => "just now".into(),
        m if m < 60 => {
            if long {
                format!("{m} min ago")
            } else {
                format!("{m}m ago")
            }
        }
        m if m < 60 * 24 => format!("{}h ago", m / 60),
        m => format!("{}d ago", m / (60 * 24)),
    }
}

// ---------------------------------------------------------------------------
// Sample data, mirroring the mock-up

fn at(now: DateTime<Local>, days: i64, h: u32, m: u32) -> DateTime<Local> {
    let t = (now + Duration::days(days)).with_time(NaiveTime::from_hms_opt(h, m, 0).unwrap());
    t.single().unwrap_or(now + Duration::days(days))
}

fn row(label: &str, short: &str, percent: f32, resets_at: Option<DateTime<Local>>) -> Row {
    Row { label: label.into(), short: Some(short.into()), percent, resets_at, verb: None, detail: None }
}

pub fn sample() -> UsageFile {
    let now = Local::now().with_second(0).unwrap_or_else(Local::now);
    let five_h = |mins: i64| Some(now + Duration::minutes(mins));
    UsageFile {
        providers: vec![
            Provider {
                name: "Claude".into(),
                icon: "claude".into(),
                accent: None,
                updated_at: Some(now - Duration::minutes(2)),
                active: 0,
                accounts: vec![
                    Account {
                        name: "Personal".into(),
                        plan: "Max".into(),
                        rows: vec![
                            row("5-hour", "5h", 6.0, five_h(250)),
                            row("Weekly", "wk", 32.0, Some(at(now, 3, 23, 0))),
                        ],
                        extras: vec![Extra { label: "Fable wk".into(), percent: 0.0 }],
                    },
                    Account {
                        name: "Work".into(),
                        plan: "Max".into(),
                        rows: vec![
                            row("5-hour", "5h", 14.0, five_h(130)),
                            row("Weekly", "wk", 41.0, Some(at(now, 4, 9, 0))),
                        ],
                        extras: vec![],
                    },
                ],
                note: None,
            },
            Provider {
                name: "Gemini 4 Argon".into(),
                icon: "gemini".into(),
                accent: None,
                updated_at: Some(now - Duration::minutes(1)),
                active: 0,
                accounts: vec![Account {
                    name: String::new(),
                    plan: "Google AI Pro".into(),
                    rows: vec![
                        row("Argon wk", "wk", 78.0, Some(at(now, 4, 15, 59))),
                        Row { detail: Some("Flash / Pro 5h".into()), ..row("Pool 5h", "pool", 0.0, None) },
                    ],
                    extras: vec![],
                }],
                note: Some("Gemini Flash/Pro pool: 5h **0%** · week **0%**".into()),
            },
            Provider {
                name: "GPT-6.1 Sol".into(),
                icon: "openai".into(),
                accent: None,
                updated_at: Some(now - Duration::minutes(2)),
                active: 0,
                accounts: vec![Account {
                    name: String::new(),
                    plan: "ChatGPT Plus".into(),
                    rows: vec![
                        row("5-hour", "5h", 0.0, five_h(268)),
                        row("Weekly", "wk", 2.0, Some(at(now, 6, 18, 2))),
                    ],
                    extras: vec![],
                }],
                note: Some("**3** free full resets available · next expires tomorrow 19:26".into()),
            },
            Provider {
                name: "Grok 4.7".into(),
                icon: "grok".into(),
                accent: None,
                updated_at: Some(now - Duration::minutes(3)),
                active: 0,
                accounts: vec![Account {
                    name: String::new(),
                    plan: "X Premium+".into(),
                    rows: vec![Row { verb: Some("ends".into()), ..row("Weekly", "wk", 1.0, Some(at(now, 6, 16, 31))) }],
                    extras: vec![],
                }],
                note: Some("Weekly credits pool · no 5-hour window".into()),
            },
        ],
    }
}
