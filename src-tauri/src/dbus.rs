//! Session bus service `fr.carru.elecwhat` (Show/Hide/ToggleVisibility/Visible/Quit)
//! + Unity LauncherEntry signal for the unread badge (GNOME dash-to-dock, KDE Plasma).

use crate::util;
use std::collections::HashMap;
use tauri::AppHandle;
use zbus::Variant;

const SERVICE_NAME: &str = "fr.carru.elecwhat";
const OBJECT_PATH: &str = "/fr/carru/elecwhat";

struct Service {
    app: AppHandle,
}

#[zbus::interface(name = "fr.carru.elecwhat")]
impl Service {
    fn show(&self) {
        util::window_show(&self.app);
    }
    fn hide(&self) {
        util::window_hide(&self.app);
    }
    fn visible(&self) -> bool {
        util::main_window(&self.app).and_then(|w| w.is_visible().ok()).unwrap_or(false)
    }
    fn toggle_visibility(&self) -> bool {
        util::window_toggle(&self.app)
    }
    fn quit(&self) {
        self.app.exit(0);
    }
}

pub struct Dbus {
    conn: zbus::Connection,
}

fn desktop_file() -> String {
    if let Ok(hint) = std::env::var("BAMF_DESKTOP_FILE_HINT") {
        if let Some(name) = std::path::Path::new(&hint).file_name() {
            return name.to_string_lossy().into_owned();
        }
    }
    if let Ok(id) = std::env::var("FLATPAK_ID") {
        return format!("{id}.desktop");
    }
    if std::env::var_os("SNAP").is_some() {
        return "elecwhat_elecwhat.desktop".into();
    }
    "elecwhat.desktop".into()
}

impl Dbus {
    pub async fn start(app: AppHandle) -> Option<Dbus> {
        let built = async {
            zbus::connection::Builder::session()
                .name(SERVICE_NAME)
                .serve_at(OBJECT_PATH, Service { app })
                .build()
                .await
        }
        .await;
        match built {
            Ok(conn) => {
                log::info!("DBus registered: {SERVICE_NAME}");
                Some(Dbus { conn })
            }
            Err(e) => {
                log::error!("DBus: {e}");
                None
            }
        }
    }

    pub async fn set_badge_count(&self, n: i64) {
        // https://wiki.ubuntu.com/Unity/LauncherAPI
        let mut props: HashMap<&str, Variant<'_>> = HashMap::new();
        props.insert("count", Variant::I64(n));
        props.insert("count-visible", Variant::Bool(n != 0));
        let uri = format!("application://{}", desktop_file());
        if let Err(e) = self
            .conn
            .emit_signal(None::<&str>, "/", "com.canonical.Unity.LauncherEntry", "Update", &(uri, props))
            .await
        {
            log::debug!("badge: {e}");
        }
    }
}
