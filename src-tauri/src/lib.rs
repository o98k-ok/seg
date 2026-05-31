mod claude;
mod codex;
mod sessions;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    LogicalPosition, Manager, Position, WindowEvent,
};

static SHOULD_EXIT: AtomicBool = AtomicBool::new(false);
// While true, the window won't auto-hide on focus loss. The frontend sets this
// before opening a native save dialog (which steals focus) so the popover stays
// put, then clears it afterward.
static SUPPRESS_HIDE: AtomicBool = AtomicBool::new(false);

// A non-activating NSPanel subclass. Reclassing the tray window into this lets
// the popover become key (receive clicks/scroll/keys) and float over another
// app's full-screen Space *without* activating seg — i.e. without a Space
// switch. This mirrors what AppKit's `MenuBarExtra(.window)` does natively.
#[cfg(target_os = "macos")]
objc2::define_class!(
    #[unsafe(super(objc2_app_kit::NSPanel))]
    #[name = "SegOverlayPanel"]
    struct OverlayPanel;

    impl OverlayPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }
    }
);

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    SHOULD_EXIT.store(true, Ordering::SeqCst);
    app.exit(0);
}

#[tauri::command]
fn set_hide_suppressed(v: bool) {
    SUPPRESS_HIDE.store(v, Ordering::SeqCst);
}

#[tauri::command]
fn save_png(path: String, base64_data: String) -> Result<(), String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let bytes = STANDARD
        .decode(base64_data.as_bytes())
        .map_err(|e| format!("decode failed: {e}"))?;
    std::fs::write(&path, bytes).map_err(|e| format!("write failed: {e}"))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn configure_overlay_window(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.set_visible_on_all_workspaces(true)?;
    window.set_always_on_top(true)?;

    use objc2::ClassType;
    use objc2_app_kit::{NSPanel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};

    let ns_window = window.ns_window()? as *mut NSWindow;

    // Swap the tao window's class for our non-activating panel subclass.
    //
    // SAFETY: `OverlayPanel` is an `NSPanel` subclass that adds no ivars, so its
    // ivar layout is a strict prefix of tao's `TaoWindow` (`NSWindow` + a
    // `focusable` ivar). Reinterpreting the larger allocation as the smaller
    // class accesses no out-of-bounds memory. We call the raw runtime function
    // rather than `AnyObject::set_class`, whose debug-assert requires equal
    // instance sizes (which deliberately differ here).
    unsafe {
        objc2::ffi::object_setClass(
            ns_window.cast::<objc2::runtime::AnyObject>(),
            OverlayPanel::class() as *const objc2::runtime::AnyClass,
        );
    }

    if let Some(panel) = unsafe { ns_window.cast::<NSPanel>().as_ref() } {
        // Non-activating: the panel can become key without activating seg, so
        // showing it never triggers a Space switch away from a full-screen app.
        panel.setStyleMask(panel.styleMask() | NSWindowStyleMask::NonactivatingPanel);
        panel.setFloatingPanel(true);
        // We drive hide/show ourselves via the focus-loss handler; don't let
        // AppKit hide the panel on app deactivation.
        panel.setHidesOnDeactivate(false);

        let behavior = panel.collectionBehavior()
            | NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::IgnoresCycle;
        panel.setCollectionBehavior(behavior);

        // Match AppKit's NSStatusWindowLevel so the popover behaves like a real menu bar panel.
        panel.setLevel(25);
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn order_overlay_front(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    // Deliberately do NOT call `NSApplication::activate()`. Activating seg while
    // another app is in native full-screen would switch Spaces (or fail to
    // surface the panel on the full-screen Space). Because the window is now a
    // non-activating panel, `makeKeyAndOrderFront` gives it key/focus in place.
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("default window icon missing")?;

            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .icon_as_template(true)
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
                        if !SUPPRESS_HIDE.load(Ordering::SeqCst) {
                            let _ = win_clone.hide();
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            sessions::list_sessions,
            quit_app,
            set_hide_suppressed,
            save_png
        ])
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
