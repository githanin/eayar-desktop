// Printing straight to a printer, without the system's print window: a browser
// always asks, so this is one of the native extras ADR 0002 keeps for the Rust
// side. The page lists the printers this computer knows (USB, network, Wi-Fi,
// Bluetooth: every printer added in the system's settings, with its driver),
// keeps the one chosen for documents and the one for tickets, and asks to print
// what it shows. The webview prints the page itself, with the page's print
// styles, so invoices, delivery notes and tickets are laid out by the web app.
//
// Each printer comes with its state, kept live: every few seconds the app looks
// again and tells the page when something changed (event "printers"). The
// system's own state is not enough on its own: macOS and Linux keep a printer
// that is switched off as "ready". So the app also looks at the device itself:
// a USB printer must be plugged in and on (the system's list of USB devices),
// a network printer (Wi-Fi or cable) must answer at its address. Windows
// already marks printers that are off or unplugged as offline.
//
// A printer that is off gets no job: the job would wait in the system's queue
// and come out later, when nobody expects it. Errors reach the page as short
// codes it turns into French text: "printer-not-found", "printer-offline",
// "unsupported" (no silent printing here: the page opens the print window
// instead) and "print-failed".

use serde::{Deserialize, Serialize};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Printer {
    /// What `print_page` takes: the system's own name for the printer.
    id: String,
    /// The name people see.
    name: String,
    /// The system's default printer.
    default: bool,
    status: Status,
    connection: Connection,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Ready,
    /// Printing something right now; a new job waits its turn.
    Busy,
    /// On but needs a hand: out of paper, jammed, door open, paused.
    Attention,
    /// Switched off, unplugged or unreachable.
    Offline,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Connection {
    Usb,
    /// Wi-Fi or cable: the system cannot tell which.
    Network,
    /// Shared by another computer.
    Shared,
    Bluetooth,
    /// Not a machine: "Microsoft Print to PDF" and the like.
    Virtual,
    Other,
}

/// The sheet to print on: its size and the same margin all round, in millimetres
/// (A4 210 × 297, A5 148 × 210, an 80 mm ticket roll…). Without one, the printer's own
/// paper and margins.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Paper {
    width_mm: f64,
    height_mm: f64,
    margin_mm: f64,
}

impl Paper {
    /// On its side: wider than tall (A5 landscape, 210 × 148). A printer is sent the sheet
    /// upright with the landscape orientation, as its driver expects.
    fn landscape(&self) -> bool {
        self.width_mm > self.height_mm
    }

    /// The sheet upright: (short side, long side).
    fn upright(&self) -> (f64, f64) {
        if self.landscape() {
            (self.height_mm, self.width_mm)
        } else {
            (self.width_mm, self.height_mm)
        }
    }

    /// The standard name of this sheet when it is one (ISO A4, A5, A6), in the usual
    /// spellings: (PPD name, PWG name). A printer is then sent the paper it knows by name,
    /// not a custom size: some drivers (Canon's CAPT…) place a custom size as if on A4, so an
    /// A5 sheet came out shifted and cut on one side.
    fn standard(&self) -> Option<(&'static str, &'static str)> {
        let (width, height) = self.upright();
        match (width.round() as i64, height.round() as i64) {
            (210, 297) => Some(("A4", "iso_a4")),
            (148, 210) => Some(("A5", "iso_a5")),
            (105, 148) => Some(("A6", "iso_a6")),
            _ => None,
        }
    }
}

/// A printer as the system lists it, and what to look at to know it is really there.
struct Found {
    printer: Printer,
    check: Check,
}

#[cfg_attr(windows, allow(dead_code))]
enum Check {
    /// The system's state is all there is.
    System,
    /// Something must answer at one of these addresses.
    Address(Vec<(String, u16)>),
    /// A Bonjour name (macOS, Linux): looked up first, then it must answer.
    Bonjour { instance: String, service: String },
    /// This USB device must be plugged in and on.
    Usb { model: String, serial: Option<String> },
}

/// A USB device plugged into this computer (and on).
#[cfg_attr(windows, allow(dead_code))]
struct UsbDevice {
    product: String,
    serial: Option<String>,
}

type Done = Box<dyn FnOnce(Result<(), String>) + Send>;

/// How often the printers are looked at again.
const EVERY: Duration = Duration::from_secs(3);
/// How long a network printer has to answer (one that is on answers in milliseconds).
const ANSWER_WITHIN: Duration = Duration::from_millis(1500);

/// The printers of this computer with their state, now.
#[tauri::command]
pub async fn printers() -> Vec<Printer> {
    tauri::async_runtime::spawn_blocking(current).await.unwrap_or_default()
}

/// Prints the page shown in the window on `printer` (an id from `printers`), without asking,
/// on `paper` when given.
#[tauri::command]
pub async fn print_page(window: tauri::WebviewWindow, printer: String, paper: Option<Paper>) -> Result<(), String> {
    // A fresh look at this printer only, just before sending.
    let id = printer.clone();
    let now = tauri::async_runtime::spawn_blocking(move || {
        let found = platform::found().into_iter().filter(|f| f.printer.id == id).collect();
        checked(found).pop()
    })
    .await
    .ok()
    .flatten();
    match now {
        None => return Err("printer-not-found".into()),
        Some(p) if p.status == Status::Offline => return Err("printer-offline".into()),
        Some(_) => {}
    }
    let (tx, mut rx) = tauri::async_runtime::channel(1);
    window
        .with_webview(move |webview| {
            platform::print(
                webview,
                &printer,
                paper,
                Box::new(move |result| {
                    let _ = tx.try_send(result);
                }),
            )
        })
        .map_err(|_| "print-failed".to_string())?;
    rx.recv().await.unwrap_or_else(|| Err("print-failed".into()))
}

/// Looks at the printers again and again, and tells the page when anything changed.
pub fn watch(app: AppHandle) {
    thread::spawn(move || {
        let mut last: Option<Vec<Printer>> = None;
        loop {
            let now = current();
            if last.as_ref() != Some(&now) {
                let _ = app.emit("printers", &now);
                last = Some(now);
            }
            thread::sleep(EVERY);
        }
    });
}

fn current() -> Vec<Printer> {
    checked(platform::found())
}

/// The printers with their real state: the system's, corrected by what the devices say.
fn checked(found: Vec<Found>) -> Vec<Printer> {
    let needs_usb = found.iter().any(|f| matches!(f.check, Check::Usb { .. }));
    let usb = if needs_usb { platform::usb_devices() } else { None };
    let (tx, rx) = mpsc::channel();
    let mut asked = Vec::new();
    let mut printers = Vec::with_capacity(found.len());
    for (i, Found { mut printer, check }) in found.into_iter().enumerate() {
        if printer.status != Status::Offline {
            match check {
                Check::System => {}
                Check::Usb { model, serial } => {
                    // Devices that cannot be read leave the system's state as it is.
                    if let Some(devices) = &usb {
                        if !plugged(&model, serial.as_deref(), devices) {
                            printer.status = Status::Offline;
                        }
                    }
                }
                Check::Address(addresses) => {
                    asked.push(i);
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let _ = tx.send((i, addresses.iter().any(|(host, port)| answers(host, *port))));
                    });
                }
                Check::Bonjour { instance, service } => {
                    asked.push(i);
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let address = platform::bonjour(&instance, &service);
                        let _ = tx.send((i, address.is_some_and(|(host, port)| answers(&host, port))));
                    });
                }
            }
        }
        printers.push(printer);
    }
    drop(tx);
    // Those that have not answered in time are off.
    let deadline = Instant::now() + ANSWER_WITHIN;
    let mut answered = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok((i, true)) => answered.push(i),
            Ok((_, false)) => {}
            Err(_) => break,
        }
    }
    for i in asked {
        if !answered.contains(&i) {
            printers[i].status = Status::Offline;
        }
    }
    printers
}

/// Whether something answers at host:port (a printer's print or web service).
fn answers(host: &str, port: u16) -> bool {
    let Ok(addresses) = (host, port).to_socket_addrs() else { return false };
    addresses.into_iter().any(|address| TcpStream::connect_timeout(&address, ANSWER_WITHIN).is_ok())
}

/// Whether the USB printer is among the devices plugged in: by serial number when both
/// have one, else by model name.
#[cfg_attr(windows, allow(dead_code))]
fn plugged(model: &str, serial: Option<&str>, devices: &[UsbDevice]) -> bool {
    let model = model.to_lowercase();
    devices.iter().any(|device| match (serial, device.serial.as_deref()) {
        (Some(serial), Some(theirs)) => serial.eq_ignore_ascii_case(theirs),
        _ => {
            let product = device.product.to_lowercase();
            !model.is_empty() && !product.is_empty() && (product.contains(&model) || model.contains(&product))
        }
    })
}

/// "%20" and the like back to text.
#[cfg_attr(windows, allow(dead_code))]
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Splits "host:port", "[v6]:port" or "host".
#[cfg_attr(windows, allow(dead_code))]
fn host_and_port(text: &str) -> (String, Option<u16>) {
    if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']').unwrap_or((rest, ""));
        return (host.to_string(), after.strip_prefix(':').and_then(|p| p.parse().ok()));
    }
    match text.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            (host.to_string(), port.parse().ok())
        }
        _ => (text.to_string(), None),
    }
}

/// How a printer is reached and how to check it, from its address in CUPS
/// ("usb://EPSON/TM-T20II?serial=…", "ipp://printer.local:631/ipp/print", "dnssd://…").
#[cfg_attr(windows, allow(dead_code))]
fn from_uri(uri: &str) -> (Connection, Check) {
    let (scheme, rest) = uri.split_once("://").unwrap_or((uri, ""));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = host_and_port(&decode(authority));
    let address = |default: u16| {
        if host.is_empty() {
            Check::System
        } else {
            Check::Address(vec![(host.clone(), port.unwrap_or(default))])
        }
    };
    match scheme {
        "usb" => {
            let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
            let model = decode(path.split_once('/').map_or("", |(_, model)| model));
            let serial = query.split('&').find_map(|pair| pair.strip_prefix("serial=")).map(decode);
            (Connection::Usb, Check::Usb { model, serial })
        }
        "ipp" | "ipps" => (Connection::Network, address(631)),
        "http" => (Connection::Network, address(80)),
        "https" => (Connection::Network, address(443)),
        "socket" => (Connection::Network, address(9100)),
        "lpd" => (Connection::Network, address(515)),
        "dnssd" | "mdns" => {
            // "Brother HL-L2350DW series._ipp._tcp.local."
            let name = decode(rest.split(['/', '?']).next().unwrap_or(""));
            match name.split_once("._") {
                Some((instance, service)) => {
                    let service = service.trim_end_matches('.').trim_end_matches(".local");
                    let check = Check::Bonjour { instance: instance.to_string(), service: format!("_{service}") };
                    (Connection::Network, check)
                }
                None => (Connection::Network, Check::System),
            }
        }
        "smb" => (Connection::Shared, address(445)),
        "bluetooth" | "bth" | "btspp" => (Connection::Bluetooth, Check::System),
        "file" | "cups-pdf" | "pdf" => (Connection::Virtual, Check::System),
        _ => (Connection::Other, Check::System),
    }
}

/// The list and the state of CUPS, the print system of macOS and Linux, through its
/// command-line tool (in English whatever the system's language).
#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod cups {
    use super::{from_uri, Check, Connection, Found, Printer, Status};
    use std::collections::HashMap;
    use std::process::Command;

    pub fn run(program: &str, args: &[&str]) -> Option<String> {
        let output = Command::new(program).args(args).env("LC_ALL", "C").output().ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Reasons a printer gives (CUPS "printer-state-reasons") that mean it is off.
    const OFF: [&str; 6] = ["offline", "offline-report", "connecting-to-device", "shutdown", "timed-out", "stopped"];
    /// Reasons that mean it is on but needs a hand.
    const HAND: [&str; 11] = [
        "media-empty",
        "media-needed",
        "media-jam",
        "door-open",
        "cover-open",
        "interlock-open",
        "toner-empty",
        "marker-supply-empty",
        "input-tray-missing",
        "output-area-full",
        "paused",
    ];

    /// One printer from `lpstat -l -p`, read line by line.
    struct Queue {
        name: String,
        status: Status,
        info: String,
        reasons: Vec<String>,
    }

    /// `display_is_id`: the name people see is also the one to print with (macOS).
    pub fn found(display_is_id: bool) -> Vec<Found> {
        let default = run("lpstat", &["-d"])
            .and_then(|out| out.split_once(':').map(|(_, name)| name.trim().to_string()))
            .filter(|name| !name.is_empty());
        // "device for NAME: URI"
        let devices: HashMap<String, String> = run("lpstat", &["-v"])
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.strip_prefix("device for "))
            .filter_map(|rest| rest.split_once(": "))
            .map(|(name, uri)| (name.to_string(), uri.trim().to_string()))
            .collect();

        let mut queues: Vec<Queue> = Vec::new();
        for line in run("lpstat", &["-l", "-p"]).unwrap_or_default().lines() {
            if let Some(rest) = line.strip_prefix("printer ") {
                // "NAME is idle.", "NAME now printing NAME-12.", "NAME disabled since …"
                let status = if rest.contains(" disabled ") {
                    Status::Offline
                } else if rest.contains(" now printing ") {
                    Status::Busy
                } else {
                    Status::Ready
                };
                let name = rest.split_whitespace().next().unwrap_or("").to_string();
                queues.push(Queue { name, status, info: String::new(), reasons: Vec::new() });
            } else if let Some(queue) = queues.last_mut() {
                let line = line.trim_start();
                if let Some(text) = line.strip_prefix("Description: ") {
                    queue.info = text.trim().to_string();
                } else if let Some(text) = line.strip_prefix("Alerts: ") {
                    queue.reasons = text.split_whitespace().filter(|r| *r != "none").map(str::to_string).collect();
                }
            }
        }

        queues
            .into_iter()
            .filter(|queue| !queue.name.is_empty())
            .map(|queue| {
                let says = |list: &[&str]| {
                    queue.reasons.iter().any(|reason| {
                        let bare = reason.trim_end_matches("-error").trim_end_matches("-warning").trim_end_matches("-report");
                        list.contains(&reason.as_str()) || list.contains(&bare)
                    })
                };
                let status = match queue.status {
                    Status::Offline => Status::Offline,
                    _ if says(&OFF) => Status::Offline,
                    _ if says(&HAND) => Status::Attention,
                    status => status,
                };
                let (connection, check) =
                    devices.get(&queue.name).map_or((Connection::Other, Check::System), |uri| from_uri(uri));
                let shown = if queue.info.is_empty() { queue.name.clone() } else { queue.info };
                Found {
                    printer: Printer {
                        id: if display_is_id { shown.clone() } else { queue.name.clone() },
                        default: default.as_deref() == Some(queue.name.as_str()),
                        name: shown,
                        status,
                        connection,
                    },
                    check,
                }
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{cups, Done, Found, Paper, UsbDevice};
    use objc2_app_kit::{NSPaperOrientation, NSPrintInfo, NSPrinter, NSPrintingPaginationMode, NSWindow};
    use objc2_foundation::{NSCopying, NSSize, NSString};
    use objc2_web_kit::WKWebView;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;
    use tauri::webview::PlatformWebview;

    pub fn found() -> Vec<Found> {
        // AppKit prints by the name people see.
        cups::found(true)
    }

    /// The USB devices plugged in, from the I/O registry.
    pub fn usb_devices() -> Option<Vec<UsbDevice>> {
        let out = cups::run("ioreg", &["-p", "IOUSB", "-l", "-w", "0"])?;
        let mut devices: Vec<UsbDevice> = Vec::new();
        for line in out.lines() {
            let line = line.trim_start_matches([' ', '|']);
            if let Some(rest) = line.strip_prefix("+-o ") {
                // "+-o TM-T20II@14200000  <class IOUSBHostDevice, …>"
                let name = rest.split('@').next().unwrap_or("").trim().to_string();
                devices.push(UsbDevice { product: name, serial: None });
            } else if let Some(device) = devices.last_mut() {
                let value = || line.split_once(" = ").map(|(_, v)| v.trim().trim_matches('"').to_string());
                if line.starts_with("\"USB Product Name\"") || line.starts_with("\"kUSBProductString\"") {
                    if let Some(v) = value() {
                        device.product = v;
                    }
                } else if line.starts_with("\"USB Serial Number\"") || line.starts_with("\"kUSBSerialNumberString\"") {
                    device.serial = value();
                }
            }
        }
        Some(devices)
    }

    /// Looks up a Bonjour printer ("dns-sd -L" keeps listening: read the first answer, then stop it).
    pub fn bonjour(instance: &str, service: &str) -> Option<(String, u16)> {
        let mut child = Command::new("dns-sd")
            .args(["-L", instance, service, "local."])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                // "… can be reached at Brother-HL.local.:631 (interface 4)"
                let Some(at) = line.split(" can be reached at ").nth(1) else { continue };
                let target = at.split_whitespace().next().unwrap_or("");
                if let Some((host, Ok(port))) = target.rsplit_once(':').map(|(h, p)| (h, p.parse::<u16>())) {
                    let _ = tx.send((host.trim_end_matches('.').to_string(), port));
                    return;
                }
            }
        });
        let answer = rx.recv_timeout(Duration::from_millis(1200)).ok();
        let _ = child.kill();
        let _ = child.wait();
        answer
    }

    pub fn print(webview: PlatformWebview, printer: &str, paper: Option<Paper>, done: Done) {
        let Some(device) = NSPrinter::printerWithName(&NSString::from_str(printer)) else {
            return done(Err("printer-not-found".into()));
        };
        // SAFETY: Tauri hands over this window's WKWebView and NSWindow, alive
        // while it runs this closure on the main thread.
        let (view, window) =
            unsafe { (&*(webview.inner() as *const WKWebView), &*(webview.ns_window() as *const NSWindow)) };
        // A copy: the shared settings stay as they are for the rest of the app.
        let info = NSPrintInfo::sharedPrintInfo().copy();
        info.setPrinter(&device);
        if let Some(paper) = paper {
            // AppKit counts in points: 72 to the inch, 25.4 mm to the inch.
            let pt = |mm: f64| mm * 72.0 / 25.4;
            // A standard sheet by the name the printer knows it (checked against the size it
            // gives for that name); any other size as a size. The sheet upright, then turned
            // when the page is on its side (0.4.4).
            let (width, height) = paper.upright();
            let named = paper.standard().and_then(|(ppd, pwg)| {
                [ppd.to_string(), ppd.to_lowercase(), pwg.replace('_', "-"), format!("{pwg}_{width}x{height}mm")]
                    .into_iter()
                    .map(|name| NSString::from_str(&name))
                    .find(|name| {
                        let size = device.pageSizeForPaper(name);
                        (size.width - pt(width)).abs() < 3.0 && (size.height - pt(height)).abs() < 3.0
                    })
            });
            match named {
                Some(name) => info.setPaperName(Some(&name)),
                None => info.setPaperSize(NSSize::new(pt(width), pt(height))),
            }
            info.setOrientation(if paper.landscape() { NSPaperOrientation::Landscape } else { NSPaperOrientation::Portrait });
            info.setTopMargin(pt(paper.margin_mm));
            info.setBottomMargin(pt(paper.margin_mm));
            info.setLeftMargin(pt(paper.margin_mm));
            info.setRightMargin(pt(paper.margin_mm));
            // The page is laid out to the sheet's width: never cut it into columns.
            info.setHorizontalPagination(NSPrintingPaginationMode::Fit);
            info.setHorizontallyCentered(false);
            info.setVerticallyCentered(false);
        }
        // SAFETY: printOperationWithPrintInfo: exists from macOS 11, Tauri's minimum.
        let operation = unsafe { view.printOperationWithPrintInfo(&info) };
        operation.setShowsPrintPanel(false);
        operation.setShowsProgressPanel(false);
        // WebKit gives its print view no size: without one the pages come out blank.
        if let Some(print_view) = operation.view() {
            print_view.setFrame(view.frame());
        }
        // Modal to the window, as WebKit needs, but with no panel the job just goes.
        // SAFETY: no delegate, so no selector or context is called back.
        unsafe {
            operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
                window,
                None,
                None,
                std::ptr::null_mut(),
            );
        }
        done(Ok(()));
    }
}

#[cfg(windows)]
mod platform {
    use super::{Check, Connection, Done, Found, Paper, Printer, Status, UsbDevice};
    use std::sync::{Arc, Mutex};
    use tauri::webview::PlatformWebview;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Environment6, ICoreWebView2PrintSettings2, ICoreWebView2_16, COREWEBVIEW2_PRINT_MEDIA_SIZE_CUSTOM,
        COREWEBVIEW2_PRINT_ORIENTATION_LANDSCAPE, COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT, COREWEBVIEW2_PRINT_STATUS,
        COREWEBVIEW2_PRINT_STATUS_PRINTER_UNAVAILABLE, COREWEBVIEW2_PRINT_STATUS_SUCCEEDED,
    };
    use webview2_com::PrintCompletedHandler;
    use windows::core::{Interface, HSTRING, PCWSTR, PWSTR};
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        DEVMODEW, DMORIENT_LANDSCAPE, DMORIENT_PORTRAIT, DM_FORMNAME, DM_IN_BUFFER, DM_ORIENTATION, DM_OUT_BUFFER,
        DM_PAPERLENGTH, DM_PAPERSIZE, DM_PAPERWIDTH,
    };
    use windows::Win32::Graphics::Printing::{
        ClosePrinter, DocumentPropertiesW, EnumPrintersW, GetDefaultPrinterW, GetPrinterW, OpenPrinterW, SetPrinterW,
        PRINTER_ACCESS_USE, PRINTER_DEFAULTSW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_HANDLE,
        PRINTER_INFO_2W, PRINTER_INFO_9W,
    };
    use windows::Win32::Storage::Xps::{DeviceCapabilitiesW, DC_PAPERNAMES, DC_PAPERS, DC_PAPERSIZE};

    // Printer status and attribute flags (winspool.h).
    const PAUSED: u32 = 0x1;
    const ERROR: u32 = 0x2;
    const PAPER_JAM: u32 = 0x8;
    const PAPER_OUT: u32 = 0x10;
    const OFFLINE: u32 = 0x80;
    const BUSY: u32 = 0x200;
    const PRINTING: u32 = 0x400;
    const NOT_AVAILABLE: u32 = 0x1000;
    const PROCESSING: u32 = 0x4000;
    const WARMING_UP: u32 = 0x10000;
    const NO_TONER: u32 = 0x40000;
    const USER_INTERVENTION: u32 = 0x100000;
    const DOOR_OPEN: u32 = 0x400000;
    const SERVER_UNKNOWN: u32 = 0x800000;
    const SERVER_OFFLINE: u32 = 0x2000000;
    const ATTRIBUTE_WORK_OFFLINE: u32 = 0x400;

    fn default_printer() -> Option<String> {
        let mut size = 0u32;
        // SAFETY: the first call only asks for the size, the second fills a buffer of that size.
        unsafe {
            let _ = GetDefaultPrinterW(None, &mut size);
            if size == 0 {
                return None;
            }
            let mut buffer = vec![0u16; size as usize];
            if !GetDefaultPrinterW(Some(PWSTR(buffer.as_mut_ptr())), &mut size).as_bool() {
                return None;
            }
            PCWSTR(buffer.as_ptr()).to_string().ok()
        }
    }

    /// How a printer is reached, from its port: "USB001", "IP_192.168.1.20", "WSD-…",
    /// "PORTPROMPT:" (Microsoft Print to PDF), or a printer of another computer ("\\PC\Printer").
    fn connection(name: &str, port: &str) -> (Connection, Check) {
        let upper = port.to_ascii_uppercase();
        if name.starts_with("\\\\") {
            let host = name.trim_start_matches('\\').split('\\').next().unwrap_or("").to_string();
            let check = if host.is_empty() { Check::System } else { Check::Address(vec![(host, 445)]) };
            return (Connection::Shared, check);
        }
        if upper.starts_with("USB") {
            return (Connection::Usb, Check::System);
        }
        if upper.starts_with("BTH") || upper.contains("BLUETOOTH") {
            return (Connection::Bluetooth, Check::System);
        }
        if upper.starts_with("WSD") {
            return (Connection::Network, Check::System);
        }
        let ip = port.strip_prefix("IP_").unwrap_or(port);
        if ip.parse::<std::net::IpAddr>().is_ok() {
            // Raw printing (9100), IPP (631), the printer's web page (80) or LPR (515): one answers.
            let addresses = [9100, 631, 80, 515].into_iter().map(|p| (ip.to_string(), p)).collect();
            return (Connection::Network, Check::Address(addresses));
        }
        let virtual_port = ["PORTPROMPT:", "FILE:", "NUL:", "NUL", "XPSPORT:", "SHRFAX:"].contains(&upper.as_str())
            || (upper.starts_with("NE") && upper.ends_with(':'));
        if virtual_port {
            return (Connection::Virtual, Check::System);
        }
        (Connection::Other, Check::System)
    }

    pub fn found() -> Vec<Found> {
        let default = default_printer();
        // Printers of this computer and the shared ones it is connected to.
        let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
        let (mut needed, mut count) = (0u32, 0u32);
        // SAFETY: the first call only asks for the size; the buffer of the second
        // is that size, aligned for the PRINTER_INFO_2W records it receives.
        unsafe {
            let _ = EnumPrintersW(flags, PCWSTR::null(), 2, None, &mut needed, &mut count);
            if needed == 0 {
                return Vec::new();
            }
            let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
            let bytes = std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), needed as usize);
            if EnumPrintersW(flags, PCWSTR::null(), 2, Some(bytes), &mut needed, &mut count).is_err() {
                return Vec::new();
            }
            std::slice::from_raw_parts(buffer.as_ptr().cast::<PRINTER_INFO_2W>(), count as usize)
                .iter()
                .filter_map(|info| {
                    let name = info.pPrinterName.to_string().ok()?;
                    let port = info.pPortName.to_string().unwrap_or_default();
                    let state = info.Status;
                    let status = if info.Attributes & ATTRIBUTE_WORK_OFFLINE != 0
                        || state & (OFFLINE | NOT_AVAILABLE | SERVER_UNKNOWN | SERVER_OFFLINE) != 0
                    {
                        Status::Offline
                    } else if state & (PAUSED | ERROR | PAPER_JAM | PAPER_OUT | NO_TONER | USER_INTERVENTION | DOOR_OPEN)
                        != 0
                    {
                        Status::Attention
                    } else if state & (BUSY | PRINTING | PROCESSING | WARMING_UP) != 0 {
                        Status::Busy
                    } else {
                        Status::Ready
                    };
                    let (connection, check) = connection(&name, &port);
                    Some(Found {
                        printer: Printer {
                            id: name.clone(),
                            default: default.as_deref() == Some(name.as_str()),
                            name,
                            status,
                            connection,
                        },
                        check,
                    })
                })
                .collect()
        }
    }

    /// Windows marks unplugged USB printers offline itself.
    pub fn usb_devices() -> Option<Vec<UsbDevice>> {
        None
    }

    /// Windows reaches Bonjour printers through its own ports (WSD), with their state.
    pub fn bonjour(_instance: &str, _service: &str) -> Option<(String, u16)> {
        None
    }

    /// An open printer, closed when dropped.
    struct Handle(PRINTER_HANDLE);

    impl Handle {
        fn open(name: &HSTRING) -> Option<Handle> {
            let defaults = PRINTER_DEFAULTSW {
                pDatatype: PWSTR::null(),
                pDevMode: std::ptr::null_mut(),
                DesiredAccess: PRINTER_ACCESS_USE,
            };
            let mut handle = PRINTER_HANDLE::default();
            // SAFETY: a printer name and a handle to fill.
            unsafe { OpenPrinterW(name, &mut handle, Some(&defaults)) }.ok()?;
            Some(Handle(handle))
        }

        /// GetPrinter at this level, in a buffer aligned for the record it receives.
        fn info(&self, level: u32) -> Option<Vec<u64>> {
            let mut needed = 0u32;
            // SAFETY: the first call only asks for the size, the second fills a buffer of that size.
            unsafe {
                let _ = GetPrinterW(self.0, level, None, &mut needed);
                if needed == 0 {
                    return None;
                }
                let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
                let bytes = std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast::<u8>(), needed as usize);
                GetPrinterW(self.0, level, Some(bytes), &mut needed).ok()?;
                Some(buffer)
            }
        }

        /// Makes `devmode` this user's default settings of the printer (none: back to the
        /// computer's default). Needs no administrator, as choosing A5 in the printer's
        /// preferences.
        fn set_user_default(&self, devmode: *mut DEVMODEW) -> bool {
            let info = PRINTER_INFO_9W { pDevMode: devmode };
            // SAFETY: a PRINTER_INFO_9W whose DEVMODE (or null) outlives the call.
            unsafe { SetPrinterW(self.0, 9, Some((&info as *const PRINTER_INFO_9W).cast()), 0) }.is_ok()
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: a handle opened by OpenPrinterW, closed once.
            unsafe {
                let _ = ClosePrinter(self.0);
            }
        }
    }

    /// A DEVMODE with its driver's own part, copied (aligned to be read back as one).
    fn copy_devmode(devmode: *const DEVMODEW) -> Option<Vec<u64>> {
        if devmode.is_null() {
            return None;
        }
        // SAFETY: a DEVMODE given by the system: dmSize + dmDriverExtra bytes long.
        unsafe {
            let len = (*devmode).dmSize as usize + (*devmode).dmDriverExtra as usize;
            let mut copy = vec![0u64; len.div_ceil(8)];
            std::ptr::copy_nonoverlapping(devmode.cast::<u8>(), copy.as_mut_ptr().cast::<u8>(), len);
            Some(copy)
        }
    }

    /// The driver's own paper of this size (within 2 mm, upright): its number and its name.
    fn driver_paper(printer: &HSTRING, port: &HSTRING, paper: &Paper) -> Option<(i16, Vec<u16>)> {
        // SAFETY: the first call counts the papers; each next one fills a buffer for that many
        // (a number, a size in tenths of a millimetre, a name of 64 characters).
        unsafe {
            let count = DeviceCapabilitiesW(printer, port, DC_PAPERS, None, None);
            if count <= 0 {
                return None;
            }
            let count = count as usize;
            let mut ids = vec![0u16; count];
            let mut sizes = vec![POINT::default(); count];
            let mut names = vec![0u16; count * 64];
            if DeviceCapabilitiesW(printer, port, DC_PAPERS, Some(PWSTR(ids.as_mut_ptr())), None) <= 0
                || DeviceCapabilitiesW(printer, port, DC_PAPERSIZE, Some(PWSTR(sizes.as_mut_ptr().cast())), None) <= 0
            {
                return None;
            }
            let _ = DeviceCapabilitiesW(printer, port, DC_PAPERNAMES, Some(PWSTR(names.as_mut_ptr())), None);
            let (upright_width, upright_height) = paper.upright();
            let (width, height) = ((upright_width * 10.0).round() as i32, (upright_height * 10.0).round() as i32);
            let off = |i: usize| (sizes[i].x - width).abs() + (sizes[i].y - height).abs();
            (0..count)
                .filter(|&i| (sizes[i].x - width).abs() <= 20 && (sizes[i].y - height).abs() <= 20)
                .min_by_key(|&i| off(i))
                .map(|i| {
                    let name = &names[i * 64..(i + 1) * 64];
                    let len = name.iter().position(|&c| c == 0).unwrap_or(64);
                    (ids[i] as i16, name[..len].to_vec())
                })
        }
    }

    /// The paper a job goes out on. WebView2 prints either on the printer's default paper or
    /// on a custom size, and some drivers leave a custom size aside: Canon's CAPT printed an A5
    /// page centred on A4, its bottom off the sheet. So the printer's own paper of that size
    /// becomes this user's default until WebView2 has sent the job, as when A5 is chosen in
    /// the printer's preferences, and the user's default is put back right after.
    struct UserPaper {
        printer: HSTRING,
        /// This user's default before (none if they never changed it).
        before: Option<Vec<u64>>,
        /// The computer's default, if a missing user default cannot be put back as missing.
        computer: Option<Vec<u64>>,
    }

    impl UserPaper {
        fn set(printer: &str, paper: &Paper) -> Option<UserPaper> {
            let name = HSTRING::from(printer);
            let handle = Handle::open(&name)?;
            let info2 = handle.info(2)?;
            // SAFETY: the buffer holds a PRINTER_INFO_2W and the strings it points to.
            let port = unsafe { (*info2.as_ptr().cast::<PRINTER_INFO_2W>()).pPortName.to_hstring() };
            let (id, form) = driver_paper(&name, &port, paper)?;
            let devmode_of = |level: u32| -> Option<Vec<u64>> {
                let info = handle.info(level)?;
                // SAFETY: levels 8 and 9 both hold one DEVMODE pointer (or null).
                copy_devmode(unsafe { (*info.as_ptr().cast::<PRINTER_INFO_9W>()).pDevMode })
            };
            let before = devmode_of(9);
            let computer = if before.is_none() { devmode_of(8) } else { None };
            // SAFETY: DocumentProperties gives its size first, then fills a buffer that large:
            // the settings in force, then those with this paper as the driver accepts them.
            unsafe {
                let size = DocumentPropertiesW(None, handle.0, &name, None, None, 0);
                if size <= 0 {
                    return None;
                }
                let mut current = vec![0u64; (size as usize).div_ceil(8)];
                let devmode = current.as_mut_ptr().cast::<DEVMODEW>();
                if DocumentPropertiesW(None, handle.0, &name, Some(devmode), None, DM_OUT_BUFFER.0) < 0 {
                    return None;
                }
                let d = &mut *devmode;
                d.dmFields |= DM_PAPERSIZE | DM_ORIENTATION;
                d.dmFields &= !(DM_PAPERLENGTH | DM_PAPERWIDTH | DM_FORMNAME);
                d.Anonymous1.Anonymous1.dmPaperSize = id;
                // The sheet upright, turned when the page is on its side (0.4.4).
                d.Anonymous1.Anonymous1.dmOrientation =
                    if paper.landscape() { DMORIENT_LANDSCAPE as i16 } else { DMORIENT_PORTRAIT as i16 };
                // The form's name too, when it fits (some drivers go by the name).
                if form.len() < d.dmFormName.len() {
                    d.dmFormName = [0; 32];
                    d.dmFormName[..form.len()].copy_from_slice(&form);
                    d.dmFields |= DM_FORMNAME;
                }
                let mut merged = vec![0u64; (size as usize).div_ceil(8)];
                let out = merged.as_mut_ptr().cast::<DEVMODEW>();
                if DocumentPropertiesW(None, handle.0, &name, Some(out), Some(devmode), (DM_IN_BUFFER | DM_OUT_BUFFER).0) < 0
                    || (*out).Anonymous1.Anonymous1.dmPaperSize != id
                {
                    return None;
                }
                if !handle.set_user_default(out) {
                    return None;
                }
            }
            Some(UserPaper { printer: name, before, computer })
        }

        /// Puts this user's default back as it was.
        fn undo(self) {
            let Some(handle) = Handle::open(&self.printer) else { return };
            let mut before = self.before;
            let as_before = before.as_mut().map_or(std::ptr::null_mut(), |b| b.as_mut_ptr().cast());
            if !handle.set_user_default(as_before) {
                if let Some(mut computer) = self.computer {
                    handle.set_user_default(computer.as_mut_ptr().cast());
                }
            }
        }
    }

    pub fn print(webview: PlatformWebview, printer: &str, paper: Option<Paper>, done: Done) {
        // The printer's own paper of that size, until the job is sent.
        let user_paper = paper.and_then(|p| UserPaper::set(printer, &p));
        let on_driver_paper = user_paper.is_some();
        let user_paper = Arc::new(Mutex::new(user_paper));
        // Called by WebView2 when the job is done, or here if it never starts.
        let done = Arc::new(Mutex::new(Some(done)));
        let finish = {
            let done = done.clone();
            move |result: Result<(), String>| {
                if let Some(paper) = user_paper.lock().ok().and_then(|mut p| p.take()) {
                    paper.undo();
                }
                if let Some(done) = done.lock().ok().and_then(|mut d| d.take()) {
                    done(result);
                }
            }
        };
        // Printing with a printer name prints without a dialog (WebView2 1.0.1518 and later).
        // SAFETY: COM calls on the webview's own thread (Tauri runs this on the main thread).
        let core = unsafe { webview.controller().CoreWebView2() }.and_then(|c| c.cast::<ICoreWebView2_16>());
        let Ok(core) = core else {
            return finish(Err("unsupported".into()));
        };
        let on_completed = finish.clone();
        let handler = PrintCompletedHandler::create(Box::new(move |result, status: COREWEBVIEW2_PRINT_STATUS| {
            on_completed(match (result, status) {
                (Ok(()), COREWEBVIEW2_PRINT_STATUS_SUCCEEDED) => Ok(()),
                (_, COREWEBVIEW2_PRINT_STATUS_PRINTER_UNAVAILABLE) => Err("printer-offline".into()),
                _ => Err("print-failed".into()),
            });
            Ok(())
        }));
        // SAFETY: as above.
        let started = (|| unsafe {
            let settings = webview.environment().cast::<ICoreWebView2Environment6>()?.CreatePrintSettings()?;
            // WebView2 counts in inches and starts from US Letter: documents here are A4.
            let inch = |mm: f64| mm / 25.4;
            let paper = paper.unwrap_or(Paper { width_mm: 210.0, height_mm: 297.0, margin_mm: 10.0 });
            // The sheet upright, turned when the page is on its side (0.4.4).
            let (width, height) = paper.upright();
            settings.SetPageWidth(inch(width))?;
            settings.SetPageHeight(inch(height))?;
            settings.SetOrientation(if paper.landscape() {
                COREWEBVIEW2_PRINT_ORIENTATION_LANDSCAPE
            } else {
                COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT
            })?;
            settings.SetMarginTop(inch(paper.margin_mm))?;
            settings.SetMarginBottom(inch(paper.margin_mm))?;
            settings.SetMarginLeft(inch(paper.margin_mm))?;
            settings.SetMarginRight(inch(paper.margin_mm))?;
            settings.SetShouldPrintBackgrounds(true)?;
            settings.SetShouldPrintHeaderAndFooter(false)?;
            let settings = settings.cast::<ICoreWebView2PrintSettings2>()?;
            // On the driver's own paper, WebView2 takes the printer's default (set above).
            // Without one of that size, a custom size: the page's own, which most drivers
            // follow. A4 without it stays on the printer's default, as before.
            if !on_driver_paper && paper.standard().map(|(name, _)| name) != Some("A4") {
                settings.SetMediaSize(COREWEBVIEW2_PRINT_MEDIA_SIZE_CUSTOM)?;
            }
            settings.SetPrinterName(&HSTRING::from(printer))?;
            core.Print(&settings, &handler)
        })();
        if started.is_err() {
            finish(Err("print-failed".into()));
        }
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod platform {
    use super::{cups, Done, Found, Paper, UsbDevice};
    use std::{cell::RefCell, fs, rc::Rc};
    use tauri::webview::PlatformWebview;
    use webkit2gtk::{PrintOperation, PrintOperationExt};

    pub fn found() -> Vec<Found> {
        // GTK prints by the CUPS queue's name.
        cups::found(false)
    }

    /// The USB devices plugged in, from /sys.
    pub fn usb_devices() -> Option<Vec<UsbDevice>> {
        let read = |path: std::path::PathBuf| fs::read_to_string(path).ok().map(|s| s.trim().to_string());
        let devices = fs::read_dir("/sys/bus/usb/devices")
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let product = read(entry.path().join("product"))?;
                Some(UsbDevice { product, serial: read(entry.path().join("serial")) })
            })
            .collect();
        Some(devices)
    }

    /// Looks up a Bonjour printer through Avahi's tools, when they are installed.
    pub fn bonjour(instance: &str, service: &str) -> Option<(String, u16)> {
        // "=;eth0;IPv4;Brother\032HL…;_ipp._tcp;local;host.local;192.168.1.5;631;…"
        let out = cups::run("avahi-browse", &["-rpt", service])?;
        let escaped = instance.replace(' ', "\\032");
        out.lines().filter(|line| line.starts_with('=')).find_map(|line| {
            let fields: Vec<&str> = line.split(';').collect();
            if fields.len() > 8 && (fields[3] == escaped || fields[3] == instance) {
                Some((fields[7].to_string(), fields[8].parse().ok()?))
            } else {
                None
            }
        })
    }

    pub fn print(webview: PlatformWebview, printer: &str, paper: Option<Paper>, done: Done) {
        let operation = PrintOperation::new(&webview.inner());
        let settings = gtk::PrintSettings::new();
        settings.set_printer(printer);
        operation.set_print_settings(&settings);
        if let Some(paper) = paper {
            // A standard sheet by its name (the driver then knows it), any other size as a size;
            // upright, turned when the page is on its side (0.4.4).
            let (width, height) = paper.upright();
            let size = match paper.standard() {
                Some((_, pwg)) => gtk::PaperSize::new(Some(pwg)),
                None => gtk::PaperSize::new_custom("adat", "Adat", width, height, gtk::Unit::Mm),
            };
            let setup = gtk::PageSetup::new();
            setup.set_paper_size(&size);
            setup.set_orientation(if paper.landscape() { gtk::PageOrientation::Landscape } else { gtk::PageOrientation::Portrait });
            setup.set_top_margin(paper.margin_mm, gtk::Unit::Mm);
            setup.set_bottom_margin(paper.margin_mm, gtk::Unit::Mm);
            setup.set_left_margin(paper.margin_mm, gtk::Unit::Mm);
            setup.set_right_margin(paper.margin_mm, gtk::Unit::Mm);
            operation.set_page_setup(&setup);
        }
        let done = Rc::new(RefCell::new(Some(done)));
        let on_failed = done.clone();
        operation.connect_failed(move |_, _| {
            if let Some(done) = on_failed.borrow_mut().take() {
                done(Err("print-failed".into()));
            }
        });
        operation.connect_finished(move |_| {
            if let Some(done) = done.borrow_mut().take() {
                done(Ok(()));
            }
        });
        // print() sends the job with these settings; run_dialog() would ask.
        operation.print();
    }
}
