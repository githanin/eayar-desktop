// Where the window may go (security audit of 13 Oct 2026; adat.app left out 25 Oct 2026). It shows
// only Ayar — ayar.ma and its subdomains (app.ayar.ma, the app; api.ayar.ma, the database's
// sign-in), and eayar.com's, its addresses until 20 Nov 2026 — and Google's sign-in, which comes
// back to the app. adat.app is not eayar's
// any more (another product since 20 Oct 2026): it opens in the browser like any other site. Any
// other address, from a link or from a page sending the window elsewhere, opens in the computer's
// own browser, with its address bar, instead of inside eayar's window where it would look like
// eayar.
//
// A link or a script opening a new window (a customer's WhatsApp, a picture in its own tab…) never
// gets a second window of the app: such a window would be the webview's own, made without these
// checks, free to go anywhere afterwards. Its address opens in the browser instead, eayar's own
// included.
//
// The page's commands (printing, updates) are allowed to https://app.ayar.ma only anyway
// (capabilities/desktop.json): another site shown here, app.eayar.com included, could not use them.

use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{Runtime, Url};

/// Where an address goes.
#[derive(Debug, PartialEq, Eq)]
enum Go {
    /// Shown in eayar's window.
    Window,
    /// Opened in the computer's browser (or its mail app, its phone app).
    Browser,
    /// Nowhere: an address no app should be handed (javascript:, data:, file:…).
    Nowhere,
}

/// The window's navigation: true keeps it in the window, false opens it in the browser instead.
pub fn navigate(url: &Url) -> bool {
    follow(url, navigation(url))
}

/// A link or a script opening a new window: never a window of the app.
pub fn new_window<R: Runtime>(url: Url, _features: NewWindowFeatures) -> NewWindowResponse<R> {
    follow(&url, popup(&url));
    NewWindowResponse::Deny
}

fn navigation(url: &Url) -> Go {
    if stays(url) {
        Go::Window
    } else {
        outside(url)
    }
}

/// A new window is never the app's, even for eayar's own pages: the same check as any address
/// that leaves the window.
fn popup(url: &Url) -> Go {
    outside(url)
}

fn outside(url: &Url) -> Go {
    if matches!(url.scheme(), "https" | "http" | "mailto" | "tel") {
        Go::Browser
    } else {
        Go::Nowhere
    }
}

fn follow(url: &Url, go: Go) -> bool {
    if go == Go::Browser {
        platform::open(url.as_str());
    }
    go == Go::Window
}

fn stays(url: &Url) -> bool {
    // `cargo tauri dev` shows studio from this computer (devUrl).
    if cfg!(debug_assertions) && url.host_str() == Some("localhost") {
        return true;
    }
    url.scheme() == "about" || eayar(url) || (url.scheme() == "https" && url.host_str().is_some_and(google_sign_in))
}

/// ayar.ma and eayar.com, and their subdomains, over https.
fn eayar(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some_and(|host| ["ayar.ma", "eayar.com"].iter().any(|domain| ours(host, domain)))
}

/// The domain itself or one of its subdomains.
fn ours(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).and_then(|sub| sub.strip_suffix('.')).is_some_and(|sub| !sub.is_empty())
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

    fn url(address: &str) -> Url {
        Url::parse(address).unwrap()
    }

    fn stays_at(address: &str) -> bool {
        stays(&url(address))
    }

    #[test]
    fn eayar_stays() {
        assert!(stays_at("https://app.ayar.ma/business/x"));
        assert!(stays_at("https://ayar.ma/"));
        assert!(stays_at("https://www.ayar.ma/parts"));
        assert!(stays_at("https://api.ayar.ma/auth/v1/authorize?provider=google"));
        assert!(stays_at("https://app.eayar.com/business/x"));
        assert!(stays_at("https://eayar.com/"));
        assert!(stays_at("https://www.eayar.com/parts"));
        assert!(stays_at("https://api.eayar.com/auth/v1/authorize?provider=google"));
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
    fn adat_app_is_not_eayar() {
        // Another product since 20 Oct 2026: the browser, like any other site.
        for address in [
            "https://adat.app/dashboard/business/x",
            "https://api.adat.app/auth/v1/authorize?provider=google",
            "https://nnpa.adat.app/",
            "https://ap.adat.app/x.webp",
        ] {
            assert!(!stays_at(address), "{address}");
            assert_eq!(navigation(&url(address)), Go::Browser, "{address}");
        }
    }

    #[test]
    fn elsewhere_leaves() {
        assert!(!stays_at("https://wa.me/212600000000"));
        assert!(!stays_at("https://evil-eayar.com/"));
        assert!(!stays_at("https://evilayar.ma/"));
        assert!(!stays_at("https://ayar.ma.evil.com/"));
        assert!(!stays_at("https://app.ayar.ma.evil.com/"));
        assert!(!stays_at("https://.ayar.ma/"));
        assert!(!stays_at("http://app.ayar.ma/"));
        assert!(!stays_at("https://eayar.com.evil.com/"));
        assert!(!stays_at("https://app.eayar.com.evil.com/"));
        assert!(!stays_at("https://.eayar.com/"));
        assert!(!stays_at("http://app.eayar.com/"));
        assert!(!stays_at("https://accounts.google.evil.com/"));
        assert!(!stays_at("https://www.google.co.evil.com/"));
        assert!(!stays_at("file:///etc/passwd"));
        assert_eq!(navigation(&url("https://wa.me/212600000000")), Go::Browser);
        assert_eq!(navigation(&url("mailto:contact@example.com")), Go::Browser);
        assert_eq!(navigation(&url("tel:+212600000000")), Go::Browser);
    }

    #[test]
    fn nothing_handed_to_other_apps_but_web_mail_and_phone() {
        for address in ["file:///etc/passwd", "javascript:alert(1)", "data:text/html,<b>x</b>", "blob:https://app.eayar.com/1"] {
            assert_eq!(navigation(&url(address)), Go::Nowhere, "{address}");
            assert_eq!(popup(&url(address)), Go::Nowhere, "{address}");
        }
    }

    #[test]
    fn popups_never_open_a_window_of_the_app() {
        // eayar's own pages and Google's sign-in too: a second window would escape these checks.
        for address in [
            "https://app.ayar.ma/api/picture?t=x",
            "https://app.eayar.com/api/picture?t=x",
            "https://eayar.com/apercu/magasin",
            "https://accounts.google.com/o/oauth2/v2/auth",
            "https://adat.app/",
            "https://wa.me/212600000000",
            "mailto:contact@example.com",
        ] {
            assert_eq!(popup(&url(address)), Go::Browser, "{address}");
        }
        assert_eq!(popup(&url("about:blank")), Go::Nowhere);
    }
}
