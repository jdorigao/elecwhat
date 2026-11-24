fn main() {
    // Only these commands may be called from the (remote) web.whatsapp.com page,
    // see capabilities/remote.json
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "notify",
            "window_toggle",
            "favicon_updated",
            "menu_toggle",
        ]),
    ))
    .expect("failed to run tauri-build");
}
