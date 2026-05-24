mod claude;
mod codex;
mod sessions;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    LogicalPosition, Manager, Position, WindowEvent,
};

static SHOULD_EXIT: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
fn configure_overlay_window(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.set_visible_on_all_workspaces(true)?;
    window.set_always_on_top(true)?;

    let ns_window = window.ns_window()? as *mut objc2_app_kit::NSWindow;
    if let Some(ns_window) = unsafe { ns_window.as_ref() } {
        use objc2_app_kit::NSWindowCollectionBehavior;

        let behavior = ns_window.collectionBehavior()
            | NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::IgnoresCycle;
        ns_window.setCollectionBehavior(behavior);

        // Match AppKit's NSStatusWindowLevel so the popover behaves like a real menu bar panel.
        ns_window.setLevel(25);
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn order_overlay_front(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        app.unhide(None);
        app.activate();
    }

    let ns_window = window.ns_window()? as *mut objc2_app_kit::NSWindow;
    if let Some(ns_window) = unsafe { ns_window.as_ref() } {
        ns_window.orderFrontRegardless();
        ns_window.makeKeyAndOrderFront(None);
    }

    Ok(())
}

fn toggle_window(window: &tauri::WebviewWindow, tray_rect: tauri::Rect) {
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let outer = window.outer_size().ok();
    let width_logical = outer.map(|s| s.width as f64 / scale).unwrap_or(420.0);

    let tray_pos = tray_rect.position.to_logical::<f64>(scale);
    let tray_size = tray_rect.size.to_logical::<f64>(scale);

    let center_x = tray_pos.x + tray_size.width / 2.0;
    let bottom_y = tray_pos.y + tray_size.height;
    let pos_x = center_x - width_logical / 2.0;
    let pos_y = bottom_y + 4.0;
    let _ = window.set_position(Position::Logical(LogicalPosition { x: pos_x, y: pos_y }));
    let _ = window.show();
    #[cfg(target_os = "macos")]
    let _ = order_overlay_front(window);
    let _ = window.set_focus();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let quit = MenuItem::with_id(app, "quit", "Quit seg", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("default window icon missing")?;

            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .icon_as_template(true)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id.as_ref() == "quit" {
                        SHOULD_EXIT.store(true, Ordering::SeqCst);
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            toggle_window(&window, rect);
                        }
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                configure_overlay_window(&window)?;

                let win_clone = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let _ = win_clone.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![sessions::list_sessions])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !SHOULD_EXIT.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}
