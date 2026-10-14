// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod config;
mod dialogs;
mod download;
mod everything;
mod http;
mod icon_image;
mod icons;
mod input;
mod launcher;
mod search;
mod system;
mod theme;
mod view;
mod win;

use futures::StreamExt;
use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, point, px, size,
};

use crate::{
    config::Config,
    launcher::Launcher,
    system::{AppEvent, HotkeyService, TrayHolder},
    theme::Theme,
};

fn main() {
    if let win::Instance::Secondary = win::acquire_single_instance() {
        // The running instance has been asked to show itself.
        return;
    }
    let start_hidden = std::env::args().any(|arg| arg == "--hidden");
    let config = Config::load();

    Application::new()
        .with_assets(icons::Assets)
        .run(move |cx: &mut App| {
            input::bind_keys(cx);
            launcher::bind_keys(cx);
            cx.set_global(Theme::from_settings(&config.settings, gpui::WindowAppearance::Light));

            let (tx, mut rx) = futures::channel::mpsc::unbounded::<AppEvent>();

            let mut hotkeys = HotkeyService::new(tx.clone());
            let hotkey_error = hotkeys.apply(&config.settings.hotkey).err();
            cx.set_global(hotkeys);
            cx.set_global(TrayHolder(system::create_tray(tx.clone())));
            win::listen_for_show_requests({
                let tx = tx.clone();
                move || {
                    let _ = tx.unbounded_send(AppEvent::Show);
                }
            });

            // Fit the window to the work area so high-DPI / high-scaling setups
            // (e.g. 1080p at 150% = 1280x720 logical) still see the whole dashboard.
            let bounds = match win::primary_work_area() {
                Some((x, y, w, h)) => {
                    let (width, height) = (1040f32.min(w * 0.92), 880f32.min(h * 0.92));
                    Bounds::new(
                        point(px(x + (w - width) / 2.), px(y + (h - height) / 2.)),
                        size(px(width), px(height)),
                    )
                }
                None => Bounds::centered(None, size(px(1040.), px(800.)), cx),
            };
            let blur = config.settings.blur_background;
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(TitlebarOptions {
                            title: Some("LovelyLauncher".into()),
                            appears_transparent: true,
                            traffic_light_position: None,
                        }),
                        window_background: if blur {
                            WindowBackgroundAppearance::Blurred
                        } else {
                            WindowBackgroundAppearance::Opaque
                        },
                        // Small enough for the compact search bar.
                        window_min_size: Some(size(px(420.), px(56.))),
                        show: !start_hidden,
                        focus: !start_hidden,
                        app_id: Some("LovelyLauncher".into()),
                        ..Default::default()
                    },
                    move |window, cx| cx.new(|cx| Launcher::new(config, hotkey_error, window, cx)),
                )
                .expect("failed to open the launcher window");

            cx.spawn(async move |cx| {
                while let Some(event) = rx.next().await {
                    let delivered = window.update(cx, |launcher, window, cx| {
                        launcher.handle_app_event(event, window, cx)
                    });
                    if delivered.is_err() {
                        break;
                    }
                }
            })
            .detach();
        });
}
