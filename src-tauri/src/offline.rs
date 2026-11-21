// No connection (Hanin, 9 Nov 2026: « mnin kimchi l-conx f desktop ktl3 dik page error dyal browser
// okt7chmni m3a khdama, wla sf7a byda — khs tl3 page zwina fiha bli kichr7 conx mkynch, o kitsna conx,
// ghir kji ktrj3 dkchi blasto »): when eayar's page cannot be reached — the app opened with no
// internet, or a page asked while it was down — the window shows eayar's own page instead of the
// webview's error page: no connection, eayar comes back by itself. It tries the address again every
// few seconds and goes back to it as soon as it answers.
//
// Windows only (WebView2: a navigation that failed for the network). While the page is open and the
// connection drops, the page itself says so (studio, components/connection-watch.tsx).

/// The page shown, the address to come back to put in it. French, as everything users see.
#[cfg(any(windows, test))]
fn page(back_to: &str) -> String {
    // A JSON string, its « < » escaped: nothing in the address can close the script.
    let target = serde_json::to_string(back_to)
        .unwrap_or_else(|_| "\"https://app.ayar.ma\"".into())
        .replace('<', "\\u003c");
    format!(
        r##"<!doctype html>
<html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>eayar</title>
<style>
  html,body{{height:100%;margin:0}}
  body{{display:flex;align-items:center;justify-content:center;background:#fafafa;color:#18181b;
    font:15px/1.5 "Segoe UI",system-ui,sans-serif;-webkit-user-select:none;user-select:none}}
  .card{{display:flex;flex-direction:column;align-items:center;gap:12px;padding:32px 40px;border:1px solid #e4e4e7;
    border-radius:14px;background:#fff;box-shadow:0 8px 24px rgba(0,0,0,.06);text-align:center;max-width:360px}}
  .mark{{width:44px;height:44px}}
  h1{{font-size:17px;font-weight:600;margin:4px 0 0}}
  p{{margin:0;color:#71717a;font-size:14px}}
  .dots{{display:flex;gap:6px;margin-top:6px}}
  .dots i{{width:6px;height:6px;border-radius:50%;background:#d4d4d8;animation:b 1.2s ease-in-out infinite}}
  .dots i:nth-child(2){{animation-delay:.2s}}.dots i:nth-child(3){{animation-delay:.4s}}
  @keyframes b{{0%,80%,100%{{opacity:.3}}40%{{opacity:1}}}}
  @media (prefers-color-scheme:dark){{body{{background:#09090b;color:#fafafa}}.card{{background:#18181b;border-color:#27272a}}p{{color:#a1a1aa}}}}
</style></head>
<body><div class="card" role="alert">
  <svg class="mark" viewBox="0 0 24 24" fill="none" stroke="#DC2626" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">
    <path d="M2 8.8a15 15 0 0 1 4.2-2.6M9.6 5.2A15 15 0 0 1 22 8.8M5 12.4a10 10 0 0 1 3.4-2M12.6 10a10 10 0 0 1 6.4 2.4M8.5 16a5 5 0 0 1 7 0M12 20h.01M3 3l18 18"/>
  </svg>
  <h1>Pas de connexion internet</h1>
  <p>eayar reprend dès que la connexion revient.</p>
  <div class="dots" aria-hidden="true"><i></i><i></i><i></i></div>
</div>
<script>
  const back = {target};
  let trying = false;
  async function again() {{
    if (trying) return;
    trying = true;
    try {{
      await fetch("https://app.ayar.ma/adat-loader.js?ping=" + Date.now(), {{ mode: "no-cors", cache: "no-store" }});
      location.replace(back);
    }} catch (e) {{
      trying = false;
    }}
  }}
  setInterval(again, 3000);
  addEventListener("online", again);
</script>
</body></html>"##
    )
}

/// The address to come back to: eayar's page that failed, else the app's home.
#[cfg(any(windows, test))]
fn back_to(failed: &str) -> String {
    match tauri::Url::parse(failed) {
        Ok(url) if url.scheme() == "https" && matches!(url.host_str(), Some("app.ayar.ma" | "app.eayar.com")) => failed.to_string(),
        _ => "https://app.ayar.ma/".to_string(),
    }
}

/// Watches the window's navigations: one that failed for the network shows the page above.
#[cfg(windows)]
pub fn watch(window: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT, COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET, COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED,
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED, COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE,
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT,
    };
    use webview2_com::{take_pwstr, NavigationCompletedEventHandler};
    use windows::core::{HSTRING, PWSTR};

    let _ = window.with_webview(|webview| {
        // SAFETY: COM calls on the webview's own thread (Tauri runs this closure on the main thread).
        let Ok(core) = (unsafe { webview.controller().CoreWebView2() }) else { return };
        let handler = NavigationCompletedEventHandler::create(Box::new(|sender, args| {
            let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
            let mut ok = windows::core::BOOL::default();
            unsafe { args.IsSuccess(&mut ok)? };
            if ok.as_bool() {
                return Ok(());
            }
            let mut status = Default::default();
            unsafe { args.WebErrorStatus(&mut status)? };
            // The network only: not a navigation stopped on purpose (an address sent to the browser).
            let network = [
                COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT,
                COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED,
                COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET,
                COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED,
                COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED,
                COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE,
                COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT,
            ];
            if !network.contains(&status) {
                return Ok(());
            }
            let mut source = PWSTR::null();
            unsafe { sender.Source(&mut source)? };
            let failed = take_pwstr(source);
            let html = HSTRING::from(page(&back_to(&failed)));
            unsafe { sender.NavigateToString(&html)? };
            Ok(())
        }));
        let mut token = 0i64;
        let _ = unsafe { core.add_NavigationCompleted(&handler, &mut token) };
    });
}

#[cfg(not(windows))]
pub fn watch(_window: &tauri::WebviewWindow) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comes_back_to_the_page_that_failed() {
        assert_eq!(back_to("https://app.ayar.ma/business/x/y/sales"), "https://app.ayar.ma/business/x/y/sales");
        assert_eq!(back_to("https://app.eayar.com/business/x/y/sales"), "https://app.eayar.com/business/x/y/sales");
        assert_eq!(back_to("https://evil.example/"), "https://app.ayar.ma/");
        assert_eq!(back_to("https://app.ayar.ma.evil.com/"), "https://app.ayar.ma/");
        assert_eq!(back_to("not a url"), "https://app.ayar.ma/");
    }

    #[test]
    fn the_page_says_it_in_french_and_goes_back() {
        let html = page("https://app.ayar.ma/x\"</script>");
        assert!(html.contains("Pas de connexion internet"));
        assert!(html.contains("location.replace(back)"));
        // The address cannot close the script: one « </script> », the page's own.
        assert_eq!(html.matches("</script>").count(), 1);
    }
}
