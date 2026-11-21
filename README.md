# eayar — Desktop app (Tauri)

Targets: **Linux, macOS, Windows**.

A lightweight **Tauri v2** shell (~10 MB) whose webview loads the deployed web app
(**https://app.ayar.ma** since 0.7.0; app.eayar.com in 0.6.x, adat.app up to 0.5.x) — the desktop app has **no UI code of its own**. Every feature
shipped in `apps/web` reaches desktop automatically. See ADR 0002.

The Rust side is reserved for native extras the web platform cannot do: printing
straight to a printer (below); later, when needed, a system tray and auto-start.

## Status

✅ A Tauri v2 shell whose webview loads https://app.ayar.ma (0.7.0, the move to ayar.ma; app.eayar.com before; 0.6.0: the app became eayar, its name
and icon; 0.4.0 to 0.5.x opened adat.app, up to 0.3.4 nnpa.adat.app), with its window menu drawn by the page, a key that brings it forward from
anywhere, and printing without the print window (0.4.0). No local frontend; the page reaches only
the commands its capability grants, on app.ayar.ma only (app.eayar.com up to 0.6.1) (adat.app too up to 0.6.0; see
"Security note").

The page also makes it feel like an app, not a browser (20 Oct 2026, no update needed): no webview
menu on a right click (only fields and selected text keep copy and paste, `desktop-bar.tsx`), and an
« Actualiser » button in the top bar, just left of the printers (`reload-button.tsx`; Ctrl/Cmd + R
too).

## Printing

A browser always shows its print window. The desktop app prints straight to a printer instead,
which a shop needs for invoices, delivery notes and counter tickets (`src-tauri/src/printing.rs`):

- `printers` lists the printers the computer knows: USB, network (Wi-Fi or cable), shared and
  Bluetooth ones, once they are added in the system's settings with their driver (as for any app).
  Each comes with how it is connected, whether it is the system's default, and its state: ready,
  printing, needs a hand (paper, cover) or **off / unplugged**.
- The state is live: the app looks again and sends the page the event `printers` when anything
  changed — every 15 s while eayar is in front, every minute while another app is, never while it is
  minimized or hidden, and at once when it comes back to the front (at most once in 3 s). Up to 0.6.0
  it looked every 3 s, all day, which kept the shops' old PCs busy (Hanin, 9 Nov 2026). The system's word is not enough on macOS and Linux, which keep a printer
  that is switched off as ready, so the app also checks the device: a USB printer must be in the
  system's list of USB devices (`ioreg` on macOS, `/sys` on Linux), a network printer must answer
  at its address within 1.5 s (Bonjour names are looked up first: `dns-sd`, Avahi). Windows marks
  printers that are off or unplugged offline itself; IP printers are checked the same way.
- `print_page { printer, paper? }` prints the page shown on that printer, without asking, on the
  given paper (size and margins in mm: A4, A5, A6, a 100 × 150 label, 80 or 58 mm ticket rolls),
  after a fresh look at it: a printer that is off gets no job (it would wait in the queue and come out later,
  unexpected). The webview prints the page itself (macOS: WebKit's print operation with no panel;
  Windows: WebView2's `Print` with a printer name, A4; Linux: WebKitGTK's print operation), so the
  page's print styles lay out the document and nothing else needs to be installed.

The page keeps two choices on each computer, the printer for documents and the one for tickets,
under the printer icon of its top bar (`apps/web/studio/src/components/printer-menu.tsx`,
`src/lib/printing.ts`), each printer's row washed green when ready, red when it is off and amber
when it needs a hand, and a red dot on the icon when a chosen printer is off. Bluetooth printers the system installs as printers are listed like the others;
small Bluetooth ticket printers without a driver would need their own work later. An app from before
0.4.0 has no such commands: the page then hides the printers and uses the browser's print window.
A4, A5 and A6 go to the printer **by the paper's name** (0.4.1: macOS takes the name the printer
gives for that size, Linux the PWG name `iso_a5`…), not as a custom size: Canon's CAPT driver
(LBP6000) placed a custom A5 page as if on A4, shifted and cut on one side. Tickets and labels
keep their own size. On Windows, WebView2 prints only on the printer's default paper or on a
custom size, and the CAPT driver leaves a custom size aside too (0.4.2: the A5 page came out
centred on A4). So (0.4.3) the driver's own paper of the page's size, found among the papers it
lists, becomes this user's default for the job (as choosing A5 in the printer's preferences, no
administrator needed) and the user's default is put back once the job is sent. A size the driver
does not list goes as a custom size. A page on its side (0.4.4: a delivery note or an invoice in
landscape, chosen in the dashboard) goes as the upright sheet with the landscape orientation, on
every system; a page sent to an older app goes through the print window instead.

The code uses the system crates the webview already pulls in (`objc2-*`, `webview2-com`/`windows`,
`gtk`/`webkit2gtk`, same versions): no new library in the app.

## Layout

```
desktop/
├── package.json      # tauri CLI scripts (dev, build)
├── app-icon.svg      # icon source: eayar's symbol, white on red (regenerate: `npx tauri icon app-icon.svg`)
└── src-tauri/
    ├── tauri.conf.json   # app config: window, frontendDist=https://app.ayar.ma, bundle
    ├── tauri.macos.conf.json  # macOS copy of the window (see "The window on macOS")
    ├── icons/            # generated by `tauri icon`
    ├── src/main.rs       # Rust entry: webview shell (see ADR 0002), window, summon key
    ├── src/printing.rs   # printing straight to a printer (see "Printing")
    └── src/links.rs      # the window shows only eayar; other addresses open in the browser
```

## Building locally

With [Rust](https://rustup.rs) and Node 20+ installed
(Linux also needs the [Tauri system deps](https://v2.tauri.app/start/prerequisites/)):

```bash
cd apps/desktop
npm install
npm run tauri build   # installers in src-tauri/target/release/bundle/
```

## Releases

Pushing a tag `desktop-v*` runs `.github/workflows/desktop-release.yml`, which builds the
installers (macOS universal `.dmg`, Windows `.msi`, Linux `.deb`), signs them, attaches them to
a GitHub release with the updater artifacts (`eayar_universal.app.tar.gz` + `.sig` for macOS, `.msi` +
`.sig` for Windows) and a `latest.json`. The release notes are the tag's paragraph of
`CHANGELOG.md` (`## 0.5.0`…): write it before tagging.

The repository is private, so the app cannot read the release itself: the files go to
`apps/web/studio/public/downloads/` (`www`'s until 20 Oct 2026) by a pull request (as the download page's), and with them
`update.json` = the release's `latest.json` with its URLs rewritten to
`https://app.ayar.ma/downloads/<file>` (app.eayar.com serves the same files) (`eayar.app.tar.gz`, `eayar.msi`, `eayar.deb`; `Adat.*` up to
0.5.2). The download page keeps its own `latest.json` (version and the three installers).

0.6.0 was built in a temporary public repository, `githanin/eayar-desktop` (the same files and
workflow, with the same signing secrets), because this repository's GitHub Actions minutes ran out
until 1 Nov 2026 and a public repository's are free. It is deleted once the shop's app has updated.

## Updates

Since 0.5.0 the app updates itself (`tauri-plugin-updater`): the page ("Mise à jour" in the top
bar) checks `https://app.ayar.ma/downloads/update.json` (0.6.x: `app.eayar.com/downloads/update.json`, the same file; 0.5.x: `adat.app/downloads/update.json`,
the same file, one Worker serves both) when the app starts and every few hours, shows the version and its notes, downloads and installs the installer when asked,
and restarts the app (`tauri-plugin-process`). Windows installs with the installer's own
progress window (`installMode: passive`); macOS replaces the app in place. Linux (`.deb`) is
not covered by the updater: install the new `.deb` by hand.

Every installer is signed with the updater key, and the app installs only what that key
signed. The key pair was made with `tauri signer generate` and lives on Hanin's Mac in
`~/.tauri/` (`adat-desktop.key`, its password in `adat-desktop.key.password`, the public key
in `adat-desktop.key.pub` = `plugins.updater.pubkey` in `tauri.conf.json`). The private key
and its password are the GitHub Actions secrets `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. **Keep a copy of the two files somewhere safe**: without
them, no new version can be signed, and the installed apps would have to be reinstalled by
hand with a new key.

## Useful commands

```bash
npm run dev           # starts studio (localhost:3001) and opens the desktop window on it, live
npm run tauri build   # native installers (.dmg / .msi / .deb ...)
```

`npm run dev` first runs `pnpm install` in `apps/web` (a few seconds when nothing changed), so
new packages pulled from `main` are always there. Code changes in studio show in the
window right away. Ctrl + C stops both. Installed apps are not affected: they open
`frontendDist`.

## Full screen, the key, one eayar at a time (0.6.1)

- **Full screen** goes through the app's own command `full_screen` (`src/main.rs`; « Plein écran »
  in the page's window menu, F11, the key below): a window without a title bar that is still
  maximized keeps its page to the screen less the taskbar on Windows (tao), so full screen left a
  black band where the taskbar is. It leaves maximized first and is maximized again after. The page
  falls back on the window's own full screen in an older app.
- **The key** that brings eayar forward and sends it away: Ctrl + Shift + Space (Cmd on a Mac), and
  on Windows the **Pause** key too (Ctrl + Shift is often the keyboard's language switch). On
  Windows whether eayar is in front is asked of Windows (`GetForegroundWindow`): the window never
  said it had the keyboard (the page has it), so the key only ever « brought it forward ».
- **One eayar at a time** (`tauri-plugin-single-instance`, a few kilobytes on Windows; on Linux it
  brings D-Bus's `zbus`): opened again, the window already open comes forward instead of a second app
  (two pages and two printer watches on a 4 GB PC).

## No connection (0.6.1)

When eayar's page cannot be reached — the app opened with no internet, or a page asked while it was
down — the window shows eayar's own page (« Pas de connexion internet · eayar reprend dès que la
connexion revient ») instead of the webview's error page, tries the address again every 3 s and
goes back to it as soon as it answers (`src/offline.rs`, Windows: a WebView2 navigation that failed
for the network). While the page is open and the connection drops, the page itself says so (studio,
`components/connection-watch.tsx`).

## The move to app.ayar.ma (0.7.0)

0.7.0 opens app.ayar.ma (the move from eayar.com to ayar.ma, 20 Nov 2026). The 0.6.x apps still
open app.eayar.com and update from its `downloads/update.json`, the same file app.ayar.ma serves, so
app.eayar.com must keep serving the app until every computer runs 0.7.0: on app.eayar.com a 0.6.x
app prints and updates, and nowhere else. A browser keeps a computer's things per address, so at
0.7.0's first opening the page goes once to app.eayar.com/carry and back to app.ayar.ma/carry with
its printers, the bons' models, paper and settings (studio `src/lib/carry.ts`); the session is not
carried: the computer signs in once with the phone's QR code (the phone reads both addresses' codes
since its patch 18).

## The window on a small screen

The window opens at 1280 × 800 (`tauri.conf.json`). On a screen smaller than that (an old PC at
1024 × 768) it opened cut on the right and at the bottom — the popup's prices, totals and
« Enregistrer » were off the screen. Since 0.5.2 `fit_screen` (`src/main.rs`) compares the
window with the screen's work area at start-up and maximizes it when it does not fit.

## The window on macOS

`tauri.macos.conf.json` replaces the window settings on macOS (Tauri replaces the whole
`windows` list, so it repeats every setting: **change both files together**). Its only
difference is the user agent: on macOS the webview is Safari's engine, so it says
`Safari` before `AdatDesktop/<version>`. Without that word, tools that work around
Safari's quirks skip it — Next.js then never reloads changed styles in the window
during `npm run dev`. Pages find the app by `AdatDesktop/` in the user agent.

## Security note

Because the webview loads a remote URL, keep IPC capabilities scoped narrowly:
only expose the Rust commands the web app actually needs, and only to `app.ayar.ma`.
`capabilities/desktop.json` grants the window commands of the window menu, the two printing
commands (`allow-printers`, `allow-print-page`, from the app manifest in `build.rs`), listening
to events (for `printers`), and the updater with the restart that follows it; the summon key's
plugin has none. `main.rs`'s tests resolve that file the way Tauri does and check who gets what.

Who gets the commands (after 0.6.0; up to 0.6.0 adat.app had them too):

- **app.ayar.ma, every page** (app.eayar.com up to 0.6.1). It is the build's `frontendDist`, so Tauri treats it as the
  app's own, « local », origin, and the capability is `"local": true` with no `remote` list
  (`tauri dev`: `devUrl`, localhost:3001). Tauri cannot narrow this to some pages: a command call
  carries only the page's origin (on macOS and Windows the IPC request's `Origin` header, with no
  path), and a capability's `remote.urls` never apply to a local origin, so `"local": false` with
  `https://app.ayar.ma/business/*` would cut printing and updates everywhere instead. A check of
  the page's path in Rust would not hold either: any script of the origin can change the path
  without leaving the page (`history.pushState`). The pages that use them are the shop's
  (`/business/…`: the top bar's printers and « Mise à jour », the bons and invoices); every page
  uses the window menu and the drag bar (`desktop-bar.tsx`, in the root layout). What protects
  them is that only our code runs on app.ayar.ma (`src/lib/csp.ts`, still report-only).
- **Nothing else**: app.eayar.com since 0.7.0, ayar.ma, api.ayar.ma, eayar.com, api.eayar.com, Google's sign-in, adat.app (another product since
  20 Oct 2026) and any other site get no command, even when shown in the window.

The window shows only eayar (`src/links.rs`, since 0.5.3; eayar.com since 0.6.0, adat.app left
out after 0.6.0; ayar.ma since 0.7.0): ayar.ma and eayar.com and their subdomains, and Google's sign-in, which comes back to them.
Any other address, from a link or a page sending the window elsewhere, opens in the computer's
browser with its address bar, so another site never appears inside eayar's window looking like
eayar. Every link or script opening a new window goes to the browser too (a customer's WhatsApp,
a picture in its own tab), eayar's own pages included: the webview's own new window would be
made without these checks and could then go anywhere. The window is made in `main.rs` from
`tauri.conf.json` (`"create": false`) to set these two handlers.
