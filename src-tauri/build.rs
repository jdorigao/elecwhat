fn main() {
    // Only these commands may be called from the (remote) web.whatsapp.com page,
    // see capabilities/remote.json
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "state_get",
            "notify",
            "open_url",
            "restore",
            "window_toggle",
            "favicon_updated",
            "page_ready",
            "menu_toggle",
        ])),
    )
    .expect("failed to run tauri-build");
}
