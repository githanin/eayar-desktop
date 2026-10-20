fn main() {
    // The app's own commands (src/printing.rs) get permissions, so a capability
    // can grant them, to adat.app only (capabilities/desktop.json).
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["printers", "print_page"])),
    )
    .expect("failed to run tauri-build");
}
