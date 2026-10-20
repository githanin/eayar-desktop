// Where the window may go (security audit of 13 Oct 2026). It shows only the app — eayar.com and
// adat.app and their subdomains (app.eayar.com since 0.6.0, 20 Oct 2026; the database's sign-in at
// api.adat.app) — and Google's sign-in, which comes back to the app. Any other address, from a link or from a page sending the window elsewhere,
// opens in the computer's own browser, with its address bar, instead of inside Adat's window where
// it would look like Adat. Links opening a new window (a customer's WhatsApp…) go there too; Adat's
// own keep opening as before.
//
// The page's commands (printing, updates) are allowed to https://app.eayar.com and https://adat.app
// only anyway (capabilities/desktop.json): another site shown here could not use them.

use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{Runtime, Url};

/// The window's navigation: true keeps it in the window, false opens it in the browser instead.
pub fn navigate(url: &Url) -> bool {
    if stays(url) {
        return true;
    }
    open_outside(url);
    false
}

/// A link or a script opening a new window.
pub fn new_window<R: Runtime>(url: Url, _features: NewWindowFeatures) -> NewWindowResponse<R> {
    if adat(&url) {
        return NewWindowResponse::Allow;
    }
    open_outside(&url);
    NewWindowResponse::Deny
}

fn stays(url: &Url) -> bool {
    // `cargo tauri dev` shows studio from this computer (devUrl).
    if cfg!(debug_assertions) && url.host_str() == Some("localhost") {
        return true;
    }
    url.scheme() == "about" || adat(url) || (url.scheme() == "https" && url.host_str().is_some_and(google_sign_in))
}

fn adat(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            ["adat.app", "eayar.com"].iter().any(|ours| host == *ours || host.strip_suffix(ours).is_some_and(|sub| sub.ends_with('.')))
        })
}

/// Google's sign-in passes through accounts.google.com, accounts.youtube.com and a country's
/// Google (accounts.google.co.ma, www.google.fr…) before coming back.
fn google_sign_in(host: &str) -> bool {
    if host == "accounts.google.com" || host == "accounts.youtube.com" {
        return true;
    }
    let Some(rest) = host.strip_prefix("accounts.google.").or_else(|| host.strip_prefix("www.google.")) else {
        return false;
    };
    // A country's ending only: « com », « fr », « co.ma », « com.br » — not « evil.com ».
    match rest.split('.').collect::<Vec<_>>().as_slice() {
        [end] => *end == "com" || end.len() == 2,
        [kind, country] => matches!(*kind, "co" | "com") && country.len() == 2,
        _ => false,
    }
}

fn open_outside(url: &Url) {
    if matches!(url.scheme(), "https" | "http" | "mailto" | "tel") {
        platform::open(url.as_str());
    }
}

#[cfg(target_os = "macos")]
mod platform {
    pub fn open(url: &str) {
        let url = url.to_string();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("/usr/bin/open").arg(url).status();
        });
    }
}

#[cfg(windows)]
mod platform {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// The default browser (or WhatsApp, the mail app…), as a double-click on the address would.
    pub fn open(url: &str) {
        // SAFETY: two valid wide strings for the call; called from the window's thread, where
        // COM is set up as the shell asks.
        unsafe {
            let _ = ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub fn open(url: &str) {
        let url = url.to_string();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("xdg-open").arg(url).status();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stays_at(address: &str) -> bool {
        stays(&Url::parse(address).unwrap())
    }

    #[test]
    fn adat_stays() {
        assert!(stays_at("https://adat.app/dashboard/business/x"));
        assert!(stays_at("https://app.eayar.com/business/x"));
        assert!(stays_at("https://eayar.com/"));
        assert!(stays_at("https://api.adat.app/auth/v1/authorize?provider=google"));
        assert!(stays_at("about:blank"));
    }

    #[test]
    fn google_sign_in_stays() {
        assert!(stays_at("https://accounts.google.com/o/oauth2/v2/auth"));
        assert!(stays_at("https://accounts.youtube.com/accounts/SetSID"));
        assert!(stays_at("https://accounts.google.co.ma/accounts/SetSID"));
        assert!(stays_at("https://www.google.fr/"));
    }

    #[test]
    fn elsewhere_leaves() {
        assert!(!stays_at("https://wa.me/212600000000"));
        assert!(!stays_at("http://adat.app/dashboard"));
        assert!(!stays_at("https://adat.app.evil.com/"));
        assert!(!stays_at("https://evil-adat.app/"));
        assert!(!stays_at("https://evil-eayar.com/"));
        assert!(!stays_at("https://app.eayar.com.evil.com/"));
        assert!(!stays_at("http://app.eayar.com/"));
        assert!(!stays_at("https://accounts.google.evil.com/"));
        assert!(!stays_at("https://www.google.co.evil.com/"));
        assert!(!stays_at("file:///etc/passwd"));
    }
}
