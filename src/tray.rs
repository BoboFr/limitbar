//! Notification-area icon: a tiny bar chart with one bar per provider.

use crate::data::UsageFile;
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub const MENU_OPEN: &str = "open";
pub const MENU_REFRESH: &str = "refresh";
pub const MENU_QUIT: &str = "quit";

const SIZE: u32 = 32;

pub fn level_rgb(percent: f32, accent: [u8; 3]) -> [u8; 3] {
    if percent >= 90.0 {
        [0xF0, 0x60, 0x5D]
    } else if percent >= 70.0 {
        [0xF5, 0xA5, 0x24]
    } else {
        accent
    }
}

fn render_icon(data: &UsageFile) -> Icon {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    let mut put = |x: u32, y: u32, rgba: [u8; 4]| {
        let i = ((y * SIZE + x) * 4) as usize;
        px[i..i + 4].copy_from_slice(&rgba);
    };

    let bars: Vec<_> = data.providers.iter().take(4).collect();
    let n = bars.len().max(1) as u32;
    let gap = 2;
    let bar_w = (SIZE - 2 - gap * (n - 1)) / n;
    let (top, bottom) = (2u32, SIZE - 2);
    let full = bottom - top;

    for (i, p) in bars.iter().enumerate() {
        let x0 = 1 + i as u32 * (bar_w + gap);
        let pct = p.max_percent().clamp(0.0, 100.0);
        let fill = ((pct / 100.0) * full as f32).round().max(3.0) as u32;
        let [r, g, b] = level_rgb(pct, p.accent_rgb());
        for y in top..bottom {
            let filled = y >= bottom - fill;
            for x in x0..x0 + bar_w {
                put(x, y, if filled { [r, g, b, 255] } else { [0x9A, 0x9D, 0xA6, 90] });
            }
        }
    }
    if bars.is_empty() {
        for y in 10..22 {
            for x in 10..22 {
                put(x, y, [0x9A, 0x9D, 0xA6, 255]);
            }
        }
    }
    Icon::from_rgba(px, SIZE, SIZE).expect("valid icon")
}

fn tooltip(data: &UsageFile) -> String {
    let parts: Vec<String> = data
        .providers
        .iter()
        .map(|p| format!("{} {:.0}%", p.name, p.max_percent()))
        .collect();
    let mut s = format!("limitbar — {}", parts.join(" · "));
    if s.chars().count() > 120 {
        s = s.chars().take(117).collect::<String>() + "...";
    }
    s
}

pub fn create(data: &UsageFile) -> TrayIcon {
    let menu = Menu::new();
    let _ = menu.append_items(&[
        &MenuItem::with_id(MENU_OPEN, "Open limitbar", true, None),
        &MenuItem::with_id(MENU_REFRESH, "Refresh", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(MENU_QUIT, "Quit", true, None),
    ]);
    TrayIconBuilder::new()
        .with_icon(render_icon(data))
        .with_tooltip(tooltip(data))
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .expect("failed to create tray icon")
}

pub fn update(tray: &TrayIcon, data: &UsageFile) {
    let _ = tray.set_icon(Some(render_icon(data)));
    let _ = tray.set_tooltip(Some(tooltip(data)));
}
