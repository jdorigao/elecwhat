//! Autostart on login via an XDG autostart entry.
//! Not supported in a snap/flatpak (no write access to the real ~/.config/autostart).

use crate::util::{config_dir, home_dir};
use std::{fs, path::PathBuf};

pub const DESKTOP_NAME: &str = "elecwhat.desktop";

pub fn supported() -> bool {
    std::env::var_os("SNAP").is_none() && std::env::var_os("FLATPAK_ID").is_none()
}

fn autostart_file() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            config_dir()
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| home_dir().join(".config"))
        });
    base.join("autostart").join(DESKTOP_NAME)
}

fn quote_exec_arg(arg: &str) -> String {
    let escaped = arg.replace('%', "%%");
    if !escaped
        .chars()
        .any(|c| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c))
    {
        return escaped;
    }
    let mut out = String::from("\"");
    for c in escaped.chars() {
        if "\"`$\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn exec_command() -> String {
    let path = std::env::var("APPIMAGE")
        .ok()
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "elecwhat".into());
    quote_exec_arg(&path)
}

fn generated_entry() -> String {
    let desktop_escape = |s: &str| s.replace('\\', "\\\\");
    [
        "[Desktop Entry]",
        "Name=ElecWhat",
        &format!("Exec={}", exec_command()),
        "Terminal=false",
        "Type=Application",
        "Icon=elecwhat",
        "StartupWMClass=elecwhat",
        "GenericName=WhatsApp Client",
        "SingleMainWindow=true",
        &format!(
            "Comment={}",
            desktop_escape("Simple desktop whatsapp client for linux")
        ),
        "MimeType=x-scheme-handler/whatsapp;",
        "Categories=Network;Chat;InstantMessaging;X-WhatsApp;",
        "X-GNOME-Autostart-enabled=true",
        "",
    ]
    .join("\n")
}

/// .desktop we were launched from, or the installed one
fn find_desktop_file() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = ["GIO_LAUNCHED_DESKTOP_FILE", "BAMF_DESKTOP_FILE_HINT"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .collect();
    let mut dirs = vec![std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/share"))];
    let xdg =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(xdg.split(':').map(PathBuf::from));
    candidates.extend(
        dirs.into_iter()
            .map(|d| d.join("applications").join(DESKTOP_NAME)),
    );
    candidates
        .into_iter()
        .filter(|p| p.is_absolute())
        .find(|p| p.exists())
}

/// Copy of an installed entry without the Exec field codes (%U ...), and not disabled
fn copied_entry(file: &PathBuf) -> std::io::Result<String> {
    let text = fs::read_to_string(file)?;
    let re = regex::Regex::new(r"\s+%[fFuUdDnNickvm]").unwrap();
    let mut in_main = false;
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with("X-GNOME-Autostart-enabled=") || line.starts_with("Hidden=") {
            continue;
        }
        if line.starts_with('[') {
            in_main = line.trim() == "[Desktop Entry]";
        }
        if in_main && line.starts_with("Exec=") {
            lines.push(re.replace_all(line, "").into_owned());
        } else {
            lines.push(line.to_string());
        }
    }
    if let Some(i) = lines.iter().position(|l| l == "[Desktop Entry]") {
        lines.insert(i + 1, "X-GNOME-Autostart-enabled=true".into());
    }
    Ok(lines.join("\n"))
}

fn entry_content() -> String {
    // Use an installed entry only when it can point to this very copy
    // (an AppImage's path changes on update; in dev it would launch the system install)
    let source = if std::env::var_os("APPIMAGE").is_some() || cfg!(debug_assertions) {
        None
    } else {
        find_desktop_file()
    };
    if let Some(src) = source {
        match copied_entry(&src) {
            Ok(c) => return c,
            Err(e) => log::warn!("autostart: can't read {}: {e}", src.display()),
        }
    }
    generated_entry()
}

pub fn is_enabled() -> bool {
    supported() && autostart_file().exists()
}

pub fn set(enabled: bool) {
    if !supported() {
        log::warn!("autostart: not supported");
        return;
    }
    let file = autostart_file();
    let res = if enabled {
        fs::create_dir_all(file.parent().unwrap()).and_then(|_| fs::write(&file, entry_content()))
    } else {
        match fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    };
    match res {
        Ok(()) => log::info!(
            "autostart {} {}",
            if enabled { "enabled" } else { "disabled" },
            file.display()
        ),
        Err(e) => log::error!("autostart: {e}"),
    }
}

/// An AppImage's path changes when moved/updated: keep an existing entry pointing to the current one.
pub fn refresh() {
    if is_enabled() {
        log::info!("autostart: refreshing entry");
        set(true);
    }
}
