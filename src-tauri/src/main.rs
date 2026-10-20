// Adat desktop: a webview shell around https://adat.app (see ADR 0002).
// No UI code lives here. The system's window controls are hidden on every
// platform (Windows/Linux: no native title bar; macOS: the traffic lights are
// hidden), so the page draws one window menu at the right of its top bar
// (Réduire, Agrandir, Plein écran, Fermer) and its top bar drags the window.
//
// One key works from anywhere, even when another app is in front: it brings
// Adat forward in full screen, and pressing it again sends Adat away and gives
// the screen back to what the user was doing. Adat then feels like a second
// screen that is always ready.
//
// The page can also print straight to a printer of this computer, without the
// system's print window, and sees each printer's state live (src/printing.rs).
//
// The window shows only Adat: other addresses open in the computer's browser
// (src/links.rs).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod links;
mod printing;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Cmd + Shift + Space on a Mac, Ctrl + Shift + Space on Windows and Linux.
/// (On Linux this needs an X11 session: Wayland has no global keys.)
const SUMMON_KEY: &str = "CommandOrControl+Shift+Space";

/// Adat in front → send it away; otherwise → bring it forward in full screen.
/// Sending it away keeps it in full screen, so it comes back as it left.
fn summon_or_dismiss(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let in_front = window.is_visible().unwrap_or(false)
        && !window.is_minimized().unwrap_or(false)
        && window.is_focused().unwrap_or(false);
    if in_front {
        // Hiding (macOS) or minimizing (elsewhere) returns to the app the user
        // was in, on the desktop they were on.
        #[cfg(target_os = "macos")]
        let _ = app.hide();
        #[cfg(not(target_os = "macos"))]
        let _ = window.minimize();
    } else {
        #[cfg(target_os = "macos")]
        let _ = app.show();
        let _ = window.unminimize();
        let _ = window.show();
        if !window.is_fullscreen().unwrap_or(false) {
            let _ = window.set_fullscreen(true);
        }
        let _ = window.set_focus();
    }
}

/// Hides the red, yellow and green buttons of a macOS window. macOS shows them
/// again on some changes (full screen, resizing), so this runs again then.
#[cfg(target_os = "macos")]
fn hide_traffic_lights(window: &tauri::Window) {
    use objc2_app_kit::{NSWindow, NSWindowButton};
    let Ok(ptr) = window.ns_window() else { return };
    // SAFETY: Tauri gives this window's NSWindow, alive as long as `window`;
    // window events and setup run on the main thread, as AppKit requires.
    let ns_window = unsafe { &*(ptr as *const NSWindow) };
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = ns_window.standardWindowButton(kind) {
            button.setHidden(true);
        }
    }
}

/// On a screen smaller than the window (1280 × 800, on an old PC at 1024 × 768) the window's
/// right side and its bottom were off the screen: the popup's prices, its totals and its
/// « Enregistrer » could not be seen, and « Vider » was pressed in their place. Such a screen
/// gets the window maximized: as large as the screen, the taskbar left out.
fn fit_screen(window: &tauri::WebviewWindow) {
    let (Ok(Some(monitor)), Ok(size)) = (window.current_monitor(), window.outer_size()) else { return };
    let area = monitor.work_area().size;
    if size.width > area.width || size.height > area.height {
        let _ = window.maximize();
    }
}

fn main() {
    tauri::Builder::default()
        // Updates: the page (adat.app) checks, downloads and installs them, and restarts the app;
        // the capabilities allow exactly that.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // The window of tauri.conf.json ("create": false), made here to watch where it goes.
            if let Some(config) = app.config().app.windows.first().cloned() {
                let window = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
                    .on_navigation(links::navigate)
                    .on_new_window(links::new_window)
                    .build()?;
                #[cfg(target_os = "macos")]
                hide_traffic_lights(&window.as_ref().window());
                #[cfg(not(target_os = "macos"))]
                let _ = window.set_decorations(false);
                fit_screen(&window);
            }
            // The page cannot reach this plugin: the capabilities allow none of
            // its commands. Only this file registers the key.
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, _key, event| {
                        if event.state() == ShortcutState::Pressed {
                            summon_or_dismiss(app);
                        }
                    })
                    .build(),
            )?;
            // Another app may already own the key: Adat then works without it.
            let _ = app.global_shortcut().register(SUMMON_KEY);
            // Printers switched on and off, plugged and unplugged, reach the page live.
            printing::watch(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![printing::printers, printing::print_page])
        .on_window_event(|window, event| {
            #[cfg(target_os = "macos")]
            if matches!(
                event,
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Focused(_) | tauri::WindowEvent::ThemeChanged(_)
            ) {
                hide_traffic_lights(window);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (window, event);
        })
        .run(tauri::generate_context!())
        .expect("error while running Adat");
}
