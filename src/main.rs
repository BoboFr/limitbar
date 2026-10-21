//! limitbar — a Windows 11 notification-area widget showing AI subscription
//! usage limits. Left-click the tray icon to open the popup, right-click for
//! the menu. The AI tools installed and logged in on this PC are detected
//! automatically (see `detect.rs`); extra providers can be listed by hand in
//! `%APPDATA%\limitbar\usage.json`. `--demo` shows the mock-up's sample data.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod antigravity;
mod data;
mod detect;
mod platform;
mod tray;
mod ui;

use chrono::Local;
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tray_icon::menu::MenuEvent;
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconEvent};

/// How often detected providers are re-checked in the background.
const DETECT_EVERY: Duration = Duration::from_secs(5 * 60);

/// State shared between the egui app, the tray callbacks, the reload timer
/// (all on the main thread) and the background detection thread.
struct Shared {
    /// What the popup shows: detected + manual providers.
    data: Mutex<data::UsageFile>,
    detected: Mutex<Vec<data::Provider>>,
    manual: Mutex<Vec<data::Provider>>,
    /// Account chosen with "Swap", by provider name.
    active: Mutex<HashMap<String, usize>>,
    mtime: Mutex<Option<SystemTime>>,
    /// Set by the detection thread, consumed by the timer on the main thread.
    dirty: AtomicBool,
    detecting: AtomicBool,
    refresh_tx: OnceLock<mpsc::Sender<()>>,
    hwnd: AtomicIsize,
    ctx: OnceLock<egui::Context>,
    anchor: Mutex<Option<platform::Anchor>>,
    last_hide: Mutex<Option<Instant>>,
    pinned: AtomicBool,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

thread_local! {
    static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

fn shared() -> &'static Shared {
    SHARED.get().expect("shared state initialised")
}

fn repaint() {
    if let Some(ctx) = shared().ctx.get() {
        ctx.request_repaint();
    }
}

fn update_tray() {
    let data = shared().data.lock().unwrap().clone();
    TRAY.with(|t| {
        if let Some(tray) = t.borrow().as_ref() {
            tray::update(tray, &data);
        }
    });
}

/// Rebuilds the displayed data from detected + manual providers.
fn compose_data() {
    let s = shared();
    let mut providers = s.detected.lock().unwrap().clone();
    for p in s.manual.lock().unwrap().iter() {
        if !providers.iter().any(|d| d.name == p.name) {
            providers.push(p.clone());
        }
    }
    let active = s.active.lock().unwrap();
    for p in &mut providers {
        if let Some(&i) = active.get(&p.name) {
            p.active = i;
        }
    }
    drop(active);
    *s.data.lock().unwrap() = data::UsageFile { providers };
}

/// Like `compose_data`, then refreshes the tray icon (main thread only).
fn compose() {
    compose_data();
    update_tray();
    repaint();
}

/// Reloads the manual JSON file if it changed; with `force`, also asks the
/// detection thread to check every tool again.
fn reload(force: bool) {
    let s = shared();
    if force {
        if let Some(tx) = s.refresh_tx.get() {
            s.detecting.store(true, Ordering::Relaxed);
            let _ = tx.send(());
        }
    }
    let mtime = data::modified();
    {
        let mut last = s.mtime.lock().unwrap();
        if !force && *last == mtime {
            return;
        }
        *last = mtime;
    }
    *s.manual.lock().unwrap() = data::load().providers;
    compose();
}

/// Background thread: detects the installed AI tools now, then every
/// `DETECT_EVERY` or when Refresh is requested.
fn spawn_detection(demo: bool) {
    let (tx, rx) = mpsc::channel::<()>();
    let _ = shared().refresh_tx.set(tx);
    std::thread::spawn(move || loop {
        let s = shared();
        s.detecting.store(true, Ordering::Relaxed);
        let found = if demo { data::sample().providers } else { detect::detect_all() };
        *s.detected.lock().unwrap() = found;
        s.detecting.store(false, Ordering::Relaxed);
        s.dirty.store(true, Ordering::Relaxed);
        repaint();
        match rx.recv_timeout(DETECT_EVERY) {
            Ok(()) => while rx.try_recv().is_ok() {}, // coalesce repeated clicks
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    });
}

fn show_popup(anchor: Option<platform::Anchor>) {
    let s = shared();
    let hwnd = s.hwnd.load(Ordering::Relaxed);
    if anchor.is_some() {
        *s.anchor.lock().unwrap() = anchor;
    }
    reload(false);
    platform::position(hwnd, *s.anchor.lock().unwrap());
    platform::show(hwnd);
    repaint();
}

fn hide_popup() {
    let s = shared();
    platform::hide(s.hwnd.load(Ordering::Relaxed));
    *s.last_hide.lock().unwrap() = Some(Instant::now());
}

fn toggle_popup(anchor: Option<platform::Anchor>) {
    let s = shared();
    let hwnd = s.hwnd.load(Ordering::Relaxed);
    if platform::is_visible(hwnd) {
        hide_popup();
        return;
    }
    // Clicking the tray icon steals focus first, which already hid the popup:
    // don't immediately reopen it.
    let just_hidden = s.last_hide.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_millis(300));
    if !just_hidden {
        show_popup(anchor);
    }
}

fn quit() -> ! {
    // Remove the icon now: process::exit skips destructors.
    TRAY.with(|t| drop(t.borrow_mut().take()));
    std::process::exit(0);
}

unsafe extern "system" fn timer_proc(_: windows_sys::Win32::Foundation::HWND, _: u32, _: usize, _: u32) {
    let s = shared();
    if s.dirty.swap(false, Ordering::Relaxed) {
        compose(); // the tray icon must be updated on the main thread
    }
    reload(false);
    if platform::is_visible(s.hwnd.load(Ordering::Relaxed)) {
        repaint(); // keeps "updated 2 min ago" fresh
    }
}

struct App {
    autostart: bool,
    collapsed: HashMap<String, bool>,
    was_focused: bool,
    last_size: egui::Vec2,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::install_fonts(&cc.egui_ctx);
        let s = shared();
        let _ = s.ctx.set(cc.egui_ctx.clone());

        if let Ok(RawWindowHandle::Win32(h)) = cc.window_handle().map(|h| h.as_raw()) {
            s.hwnd.store(h.hwnd.get(), Ordering::Relaxed);
            platform::style_window(h.hwnd.get());
        }

        let tray = tray::create(&s.data.lock().unwrap());
        TRAY.with(|t| *t.borrow_mut() = Some(tray));

        TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                toggle_popup(Some(platform::Anchor {
                    x: rect.position.x as i32,
                    y: rect.position.y as i32,
                    w: rect.size.width as i32,
                    h: rect.size.height as i32,
                }));
            }
        }));
        MenuEvent::set_event_handler(Some(|event: MenuEvent| match event.id.0.as_str() {
            tray::MENU_OPEN => show_popup(None),
            tray::MENU_REFRESH => reload(true),
            tray::MENU_QUIT => quit(),
            _ => {}
        }));

        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetTimer(std::ptr::null_mut(), 0, 1000, Some(timer_proc));
        }

        // `--open`: show the popup right away (handy for testing).
        if std::env::args().any(|a| a == "--open") {
            show_popup(None);
        }

        Self {
            autostart: platform::autostart_enabled(),
            collapsed: data::load_collapsed(),
            was_focused: false,
            last_size: egui::Vec2::ZERO,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let s = shared();
        let hwnd = s.hwnd.load(Ordering::Relaxed);
        let pinned = s.pinned.load(Ordering::Relaxed);

        // Hide when clicking elsewhere, unless pinned.
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(false);
        if self.was_focused && !focused && !pinned {
            hide_popup();
        }
        self.was_focused = focused;

        let (refresh_key, quit_key, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::CTRL, egui::Key::R),
                i.consume_key(egui::Modifiers::CTRL, egui::Key::Q),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if quit_key {
            quit();
        }
        if esc {
            hide_popup();
        }
        if refresh_key {
            reload(true);
        }

        let data = s.data.lock().unwrap().clone();
        let view = ui::ViewState {
            pinned,
            autostart: self.autostart,
            detecting: s.detecting.load(Ordering::Relaxed),
            collapsed: self.collapsed.clone(),
        };
        let frame = egui::Frame::new().fill(ui::bg()).inner_margin(egui::Margin::same(ui::MARGIN));
        let mut content_h = 0.0;
        let actions = egui::CentralPanel::default()
            .frame(frame)
            .show(ctx, |ui| {
                let a = ui::draw(ui, &data, &view, Local::now());
                content_h = ui.cursor().top() - ui.max_rect().top() - ui.spacing().item_spacing.y;
                a
            })
            .inner;

        if actions.quit {
            quit();
        }
        if actions.refresh {
            reload(true);
        }
        if actions.toggle_pin {
            s.pinned.store(!pinned, Ordering::Relaxed);
        }
        if actions.toggle_autostart {
            self.autostart = !self.autostart;
            platform::set_autostart(self.autostart);
        }
        if let Some((name, collapsed)) = actions.collapse {
            self.collapsed.insert(name, collapsed);
            data::save_collapsed(&self.collapsed);
            ctx.request_repaint();
        }
        if let Some(i) = actions.swap {
            if let Some(p) = data.providers.get(i) {
                let n = p.accounts.len().max(1);
                let next = (p.active.min(n - 1) + 1) % n;
                s.active.lock().unwrap().insert(p.name.clone(), next);
                compose();
            }
        }

        // Fit the window to its content, then re-anchor it to the tray.
        let want = egui::vec2(ui::WIDTH, (content_h + 2.0 * ui::MARGIN as f32).ceil());
        let size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size())).unwrap_or(want);
        if (size.y - want.y).abs() > 1.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(want));
        }
        if size != self.last_size {
            self.last_size = size;
            if platform::is_visible(hwnd) {
                platform::position(hwnd, *s.anchor.lock().unwrap());
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ui::bg().to_normalized_gamma_f32()
    }
}

fn main() -> eframe::Result<()> {
    if !platform::single_instance() {
        return Ok(());
    }
    let _ = SHARED.set(Shared {
        data: Mutex::new(data::UsageFile::default()),
        detected: Mutex::new(Vec::new()),
        manual: Mutex::new(data::load().providers),
        active: Mutex::new(HashMap::new()),
        mtime: Mutex::new(data::modified()),
        dirty: AtomicBool::new(false),
        detecting: AtomicBool::new(true),
        refresh_tx: OnceLock::new(),
        hwnd: AtomicIsize::new(0),
        ctx: OnceLock::new(),
        anchor: Mutex::new(None),
        last_hide: Mutex::new(None),
        pinned: AtomicBool::new(false),
    });
    compose_data();
    spawn_detection(std::env::args().any(|a| a == "--demo"));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("limitbar")
            .with_inner_size([ui::WIDTH, 560.0])
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_visible(false),
        ..Default::default()
    };
    eframe::run_native("limitbar", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
