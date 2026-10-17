//! Global hotkey and tray icon. Both create hidden Win32 windows on the main
//! thread, whose messages are pumped by GPUI's message loop.

use futures::channel::mpsc::UnboundedSender;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use gpui::Global;

/// Where windows other than the launcher (the ring) send requests for it.
pub struct AppEvents(pub UnboundedSender<AppEvent>);

impl Global for AppEvents {}
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[derive(Clone, Copy, Debug)]
pub enum AppEvent {
    /// Global hotkey pressed.
    Hotkey,
    /// Tray click / menu: toggle the full window.
    Toggle,
    Show,
    Settings,
    Quit,
    /// Space / arrows / Esc in an Explorer file list (QuickLook-style preview).
    Preview(crate::preview::hook::HookEvent),
    /// Run a quick action (picked in the ring, which has no access to the launcher's window).
    Action(crate::launcher::QuickAction),
    /// The 剪贴板历史 hotkey.
    ClipboardHotkey,
}

pub struct HotkeyService {
    manager: Option<GlobalHotKeyManager>,
    current: Option<HotKey>,
    clipboard: Option<HotKey>,
}

/// Id of the registered 剪贴板历史 hotkey (0: none), read by the event handler.
static CLIPBOARD_HOTKEY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl Global for HotkeyService {}

impl HotkeyService {
    pub fn new(tx: UnboundedSender<AppEvent>) -> Self {
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state == HotKeyState::Pressed {
                let clipboard = CLIPBOARD_HOTKEY.load(std::sync::atomic::Ordering::Relaxed);
                let _ = tx.unbounded_send(if clipboard != 0 && event.id == clipboard {
                    AppEvent::ClipboardHotkey
                } else {
                    AppEvent::Hotkey
                });
            }
        }));
        Self {
            manager: GlobalHotKeyManager::new().ok(),
            current: None,
            clipboard: None,
        }
    }

    /// Register `spec` as the 剪贴板历史 hotkey (empty: none).
    pub fn apply_clipboard(&mut self, spec: &str) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        let manager = self.manager.as_ref().ok_or("无法初始化全局快捷键")?;
        if spec.trim().is_empty() {
            if let Some(old) = self.clipboard.take() {
                let _ = manager.unregister(old);
            }
            CLIPBOARD_HOTKEY.store(0, Ordering::Relaxed);
            return Ok(());
        }
        let hotkey: HotKey = normalize(spec).parse().map_err(|e| format!("快捷键格式无效：{e}"))?;
        if self.clipboard == Some(hotkey) {
            return Ok(());
        }
        if self.current == Some(hotkey) {
            return Err("和呼出快捷键重复了".into());
        }
        manager
            .register(hotkey)
            .map_err(|_| format!("快捷键 {spec} 注册失败，可能已被其他程序占用"))?;
        if let Some(old) = self.clipboard.replace(hotkey) {
            let _ = manager.unregister(old);
        }
        CLIPBOARD_HOTKEY.store(hotkey.id(), Ordering::Relaxed);
        Ok(())
    }

    /// Register `spec` (e.g. "Alt+Space") in place of the current hotkey.
    /// On failure the previous hotkey stays active.
    pub fn apply(&mut self, spec: &str) -> Result<(), String> {
        let manager = self.manager.as_ref().ok_or("无法初始化全局快捷键")?;
        let hotkey: HotKey = normalize(spec)
            .parse()
            .map_err(|e| format!("快捷键格式无效：{e}"))?;
        if self.current == Some(hotkey) {
            return Ok(());
        }
        manager
            .register(hotkey)
            .map_err(|_| format!("快捷键 {spec} 注册失败，可能已被其他程序占用"))?;
        if let Some(old) = self.current.replace(hotkey) {
            let _ = manager.unregister(old);
        }
        Ok(())
    }
}

/// Accept friendly spellings such as "alt + space" or "Ctrl+Shift+L".
fn normalize(spec: &str) -> String {
    spec.split('+')
        .map(|part| {
            let part = part.trim();
            match part.to_ascii_lowercase().as_str() {
                "win" | "windows" | "meta" => "Super".to_string(),
                "space" => "Space".to_string(),
                "esc" => "Escape".to_string(),
                "enter" | "return" => "Enter".to_string(),
                "`" | "~" | "backquote" => "Backquote".to_string(),
                _ => part.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Keeps the tray icon alive for the lifetime of the app.
pub struct TrayHolder(#[allow(dead_code)] pub Option<TrayIcon>);

impl Global for TrayHolder {}

pub fn create_tray(tx: UnboundedSender<AppEvent>) -> Option<TrayIcon> {
    let rgba = include_bytes!(concat!(env!("OUT_DIR"), "/tray_32.rgba")).to_vec();
    let icon = Icon::from_rgba(rgba, 32, 32).ok()?;

    let show = MenuItem::new("显示 / 隐藏", true, None);
    let settings = MenuItem::new("设置…", true, None);
    let quit = MenuItem::new("退出", true, None);
    let menu = Menu::new();
    menu.append_items(&[
        &show,
        &settings,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .ok()?;

    let (show_id, settings_id, quit_id) = (show.id().clone(), settings.id().clone(), quit.id().clone());
    let menu_tx = tx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let app_event = if event.id == show_id {
            AppEvent::Toggle
        } else if event.id == settings_id {
            AppEvent::Settings
        } else if event.id == quit_id {
            AppEvent::Quit
        } else {
            return;
        };
        let _ = menu_tx.unbounded_send(app_event);
    }));

    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = tx.unbounded_send(AppEvent::Toggle);
        }
    }));

    TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("LovelyLauncher")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .ok()
}
