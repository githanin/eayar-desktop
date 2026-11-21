// Ayar desktop: a webview shell around https://app.ayar.ma (see ADR 0002; app.eayar.com in
// 0.6.x, adat.app up to 0.5.x). No UI code lives here. The system's window controls are hidden on every
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
// The window shows only eayar: other addresses, and every new window, open in the
// computer's browser (src/links.rs). Only app.ayar.ma reaches the commands
// (capabilities/desktop.json).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod links;
mod offline;
mod printing;

#[cfg(not(target_os = "macos"))]
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Cmd + Shift + Space on a Mac, Ctrl + Shift + Space on Windows and Linux; on Windows the Pause
/// key too, one key that no other app takes (Ctrl + Shift is often the keyboard's language switch,
/// and Word's and Excel's key). (On Linux this needs an X11 session: Wayland has no global keys.)
#[cfg(windows)]
const SUMMON_KEYS: &[&str] = &["Control+Shift+Space", "Pause"];
#[cfg(not(windows))]
const SUMMON_KEYS: &[&str] = &["CommandOrControl+Shift+Space"];

/// Where eayar's window is: in front, behind another app, or minimized / hidden.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Place {
    Front,
    Behind,
    Away,
}

pub(crate) fn window_place(app: &AppHandle) -> Place {
    let Some(window) = app.get_webview_window("main") else { return Place::Away };
    if window.is_minimized().unwrap_or(false) || !window.is_visible().unwrap_or(true) {
        Place::Away
    } else if in_front(&window) {
        Place::Front
    } else {
        Place::Behind
    }
}

/// On Windows the keyboard's focus is the page's, not the window's: the window never says it has
/// it (tao), so the key always « brought eayar forward », which did nothing when it was already
/// there (« mra katkhdm, mra la »). The front window is asked of Windows itself.
#[cfg(windows)]
fn in_front(window: &tauri::WebviewWindow) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let Ok(hwnd) = window.hwnd() else { return false };
    // SAFETY: a plain query of the window in front.
    unsafe { GetForegroundWindow() == hwnd }
}
#[cfg(not(windows))]
fn in_front(window: &tauri::WebviewWindow) -> bool {
    window.is_focused().unwrap_or(false)
}

/// Whether the window was maximized before full screen: given back after.
#[cfg(not(target_os = "macos"))]
static MAXIMIZED_BEFORE: AtomicBool = AtomicBool::new(false);

/// Full screen on or off. A window without a title bar that is still maximized keeps its page to
/// the screen less the taskbar on Windows (tao): full screen then left a black band where the
/// taskbar is (« k7oliya t7t f blast dock »). It leaves maximized first, and is maximized again
/// after.
fn set_full_screen(window: &tauri::WebviewWindow, on: bool) {
    if window.is_fullscreen().unwrap_or(false) == on {
        return;
    }
    if on {
        #[cfg(not(target_os = "macos"))]
        {
            let maximized = window.is_maximized().unwrap_or(false);
            MAXIMIZED_BEFORE.store(maximized, Ordering::Relaxed);
            if maximized {
                let _ = window.unmaximize();
            }
        }
        let _ = window.set_fullscreen(true);
    } else {
        let _ = window.set_fullscreen(false);
        #[cfg(not(target_os = "macos"))]
        if MAXIMIZED_BEFORE.swap(false, Ordering::Relaxed) {
            let _ = window.maximize();
        }
    }
}

/// The page's « Plein écran » and F11: on, off, or the other way (`on` not given); says which.
#[tauri::command]
fn full_screen(window: tauri::WebviewWindow, on: Option<bool>) -> bool {
    let on = on.unwrap_or_else(|| !window.is_fullscreen().unwrap_or(false));
    set_full_screen(&window, on);
    on
}

/// Brings eayar's window forward as it was (a second launch of the app).
fn bring_forward(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    #[cfg(target_os = "macos")]
    let _ = app.show();
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// Adat in front → send it away; otherwise → bring it forward in full screen.
/// Sending it away keeps it in full screen, so it comes back as it left.
fn summon_or_dismiss(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let in_front = window_place(app) == Place::Front;
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
        set_full_screen(&window, true);
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
        // One eayar at a time: opened again (its icon clicked while it is minimized), the one already
        // open comes forward instead of a second one (two pages, two printer watches, on 4 GB).
        // First, as the plugin asks.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| bring_forward(app)))
        // Updates: the page (app.ayar.ma) checks, downloads and installs them, and restarts the
        // app; the capabilities allow exactly that.
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
                // No connection: eayar's own page instead of the webview's error page (src/offline.rs).
                offline::watch(&window);
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
            // Another app may already own a key: eayar then works with the other one, or without.
            for key in SUMMON_KEYS {
                let _ = app.global_shortcut().register(*key);
            }
            // Printers switched on and off, plugged and unplugged, reach the page live.
            printing::watch(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![printing::printers, printing::print_page, full_screen])
        .on_window_event(|window, event| {
            // Back in front, or out of the taskbar: the printers looked at now (they wait while
            // eayar is minimized).
            if matches!(event, tauri::WindowEvent::Focused(true) | tauri::WindowEvent::Resized(_)) {
                printing::wake(window.app_handle());
            }
            #[cfg(target_os = "macos")]
            if matches!(
                event,
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Focused(_) | tauri::WindowEvent::ThemeChanged(_)
            ) {
                hide_traffic_lights(window);
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Adat");
}

/// capabilities/desktop.json as Tauri resolves it into the app: who reaches which command.
/// The same resolver and lookup the app runs (`Resolved::resolve`, `resolve_access`); what only the
/// running app decides is that app.ayar.ma, its frontendDist, calls as « local ». A debug build
/// only: there `RuntimeAuthority::new` takes the plugins' manifests.
#[cfg(all(test, debug_assertions))]
mod capability_tests {
    use std::collections::BTreeMap;
    use tauri::ipc::{Origin, RuntimeAuthority};
    use tauri::utils::acl::{capability::Capability, manifest::Manifest, resolved::Resolved};
    use tauri::utils::platform::Target;
    use tauri::Url;

    const CAPABILITY: &str = include_str!("../capabilities/desktop.json");

    /// What the page calls: the window menu and the drag bar (studio's desktop-bar.tsx), printing
    /// (lib/printing.ts), updates and the restart after one (update-menu.tsx).
    const GRANTED: &[&str] = &[
        "plugin:window|start_dragging",
        "plugin:window|minimize",
        "plugin:window|toggle_maximize",
        "plugin:window|is_fullscreen",
        "full_screen",
        "plugin:window|close",
        "printers",
        "print_page",
        "plugin:event|listen",
        "plugin:event|unlisten",
        "plugin:updater|check",
        "plugin:updater|download_and_install",
        "plugin:process|restart",
    ];

    fn capability() -> Capability {
        serde_json::from_str(CAPABILITY).unwrap()
    }

    fn authority() -> RuntimeAuthority {
        // The plugins' permissions, written by tauri-build (build.rs) before this is compiled.
        let manifests = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/gen/schemas/acl-manifests.json"))
            .expect("gen/schemas/acl-manifests.json, written by build.rs");
        let manifests: BTreeMap<String, Manifest> = serde_json::from_str(&manifests).unwrap();
        let capability = capability();
        let resolved = Resolved::resolve(&manifests, BTreeMap::from([(capability.identifier.clone(), capability)]), Target::current())
            .unwrap();
        RuntimeAuthority::new(manifests, resolved)
    }

    fn allowed(authority: &RuntimeAuthority, command: &str, origin: &Origin) -> bool {
        authority.resolve_access(command, "main", "main", origin).is_some()
    }

    fn remote(address: &str) -> Origin {
        Origin::Remote { url: Url::parse(address).unwrap() }
    }

    #[test]
    fn only_the_app_itself() {
        // « local » is the build's frontendDist, https://app.ayar.ma (devUrl in `tauri dev`).
        let capability = capability();
        assert!(capability.local);
        assert!(capability.remote.is_none(), "no other site gets the commands");
        assert_eq!(capability.windows, ["main"]);
    }

    #[test]
    fn the_app_reaches_what_its_pages_call() {
        let authority = authority();
        for command in GRANTED {
            assert!(allowed(&authority, command, &Origin::Local), "{command}");
        }
    }

    #[test]
    fn no_other_site_reaches_anything() {
        let authority = authority();
        for address in [
            "https://adat.app/",
            "https://adat.app/business/x",
            "https://nnpa.adat.app/",
            "https://ayar.ma/",
            "https://api.ayar.ma/",
            "https://eayar.com/",
            "https://api.eayar.com/",
            "https://accounts.google.com/",
            "https://wa.me/",
            // The app's address up to 0.6.1: no command there any more.
            "https://app.eayar.com/",
            // app.ayar.ma is reached as « local » only, never through a remote list.
            "https://app.ayar.ma/",
        ] {
            for command in GRANTED {
                assert!(!allowed(&authority, command, &remote(address)), "{command} on {address}");
            }
        }
    }

    #[test]
    fn nothing_more_than_that() {
        let authority = authority();
        for command in [
            "plugin:window|set_title",
            // Full screen goes through `full_screen` (the black band), never straight.
            "plugin:window|set_fullscreen",
            "plugin:window|set_size",
            "plugin:webview|create_webview_window",
            "plugin:event|emit",
            "plugin:process|exit",
            "plugin:global-shortcut|register",
            "plugin:global-shortcut|unregister_all",
        ] {
            assert!(!allowed(&authority, command, &Origin::Local), "{command}");
        }
    }
}
