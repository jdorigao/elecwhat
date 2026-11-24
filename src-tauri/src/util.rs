use crate::AppState;
use regex::Regex;
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};
use tauri::{AppHandle, Manager, Url, WebviewWindow, image::Image};

pub const URL_SCHEME: &str = "whatsapp";

/// Same location Electron used (`app.getPath("userData")`), so existing
/// config.json / user-scripts / user-icons keep working.
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("elecwhat")
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase()
        .contains("gnome")
}

/// ["pt-BR", "en-US"], from LANGUAGE / LC_ALL / LC_MESSAGES / LANG
pub fn preferred_langs() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.split(['.', '@']).next().unwrap_or("").replace('_', "-");
        if !s.is_empty() && s != "C" && s != "POSIX" && !out.contains(&s) {
            out.push(s);
        }
    };
    if let Ok(l) = std::env::var("LANGUAGE") {
        l.split(':').for_each(&mut push);
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(l) = std::env::var(var) {
            push(&l);
        }
    }
    if out.is_empty() {
        out.push("en-US".into());
    }
    out
}

pub type Translations = HashMap<String, String>;

pub fn load_translations(lang: &str) -> Translations {
    let json = match lang.split('-').next().unwrap_or("") {
        "de" => include_str!("../../locales/de.json"),
        "fr" => include_str!("../../locales/fr.json"),
        "it" => include_str!("../../locales/it.json"),
        "pt" => include_str!("../../locales/pt.json"),
        _ => include_str!("../../locales/en.json"),
    };
    serde_json::from_str(json).unwrap_or_default()
}

pub fn tr(t: &Translations, key: &str, default: &str) -> String {
    t.get(key).cloned().unwrap_or_else(|| default.to_string())
}

// ---- window helpers --------------------------------------------------------

pub fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

pub fn window_show(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        if w.is_minimized().unwrap_or(false) {
            let _ = w.unminimize();
        }
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn window_hide(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        use tauri_plugin_window_state::{AppHandleExt, StateFlags};
        let _ =
            app.save_window_state(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED);
        let _ = w.hide();
    }
}

/// returns the new visibility
pub fn window_toggle(app: &AppHandle) -> bool {
    let visible = main_window(app)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if visible {
        window_hide(app);
    } else {
        window_show(app);
    }
    !visible
}

// ---- urls / icons ----------------------------------------------------------

/// whatsapp://send/?phone=1 -> https://web.whatsapp.com/send/?phone=1
pub fn convert_whatsapp_url(url: &str) -> Option<Url> {
    if !url.starts_with(&format!("{URL_SCHEME}:")) {
        return None;
    }
    let u = Url::parse(url).ok()?;
    // "send" is parsed as the host, not the path
    let mut web = format!(
        "https://web.whatsapp.com/{}{}",
        u.host_str().unwrap_or(""),
        u.path()
    );
    if let Some(q) = u.query() {
        web.push('?');
        web.push_str(q);
    }
    if let Some(f) = u.fragment() {
        web.push('#');
        web.push_str(f);
    }
    Url::parse(&web).ok()
}

pub fn unread_from_favicon(url: &str) -> i64 {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re =
        RE.get_or_init(|| Regex::new(r"^https://web\.whatsapp\.com/favicon/1x/f(\d+)/").unwrap());
    re.captures(url)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(0)
}

fn icon_basename(url: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"v[0-9]+/").unwrap());
    let url = re.replace(url, "");
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn decode_icon(bytes: &[u8]) -> Option<Image<'static>> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8(); // png / webp
    let (w, h) = img.dimensions();
    Some(Image::new_owned(img.into_raw(), w, h))
}

pub fn user_icon(state: &AppState, url: &str) -> Option<Image<'static>> {
    let basename = icon_basename(url);
    if let Some(cached) = state.user_icon_cache.lock().unwrap().get(&basename) {
        return cached.clone();
    }
    let path = state.icons_dir.join(format!("{basename}.png"));
    log::debug!("favicon {basename} {}", path.display());
    let icon = std::fs::read(&path).ok().and_then(|b| decode_icon(&b));
    state
        .user_icon_cache
        .lock()
        .unwrap()
        .insert(basename, icon.clone());
    icon
}

pub async fn get_icon(state: &AppState, url: &str) -> Option<Image<'static>> {
    if let Some(i) = user_icon(state, url) {
        return Some(i);
    }
    let url = url.replace("/1x/", "/2x/"); // larger images
    if let Some(cached) = state.icon_cache.lock().unwrap().get(&url) {
        return Some(cached.clone());
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"^https://(static\.whatsapp\.net|web\.whatsapp\.com)/").unwrap()
    });
    if !re.is_match(&url) {
        return None;
    }
    let client = reqwest::Client::builder()
        .user_agent(&state.user_agent)
        .build()
        .ok()?;
    let resp = match client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("fetch {url}: {e}");
            return None;
        }
    };
    let bytes = resp.bytes().await.ok()?;
    let icon = decode_icon(&bytes)?;
    state.icon_cache.lock().unwrap().insert(url, icon.clone());
    Some(icon)
}

// ---- css / scripts ---------------------------------------------------------

/// `css` config entries are either inline css, an https:// url or a file:// url
/// (file://~/x.css = relative to $HOME)
pub async fn load_css_entry(entry: &str) -> Option<String> {
    let Ok(url) = Url::parse(entry) else {
        return Some(entry.to_string());
    };
    match url.scheme() {
        "file" => {
            let mut path = PathBuf::from(url.path());
            if url.host_str() == Some("~") {
                path = home_dir().join(url.path().trim_start_matches('/'));
            }
            log::debug!("load css from file {}", path.display());
            std::fs::read_to_string(path).ok()
        }
        "https" => {
            log::debug!("load css from {url}");
            let resp = reqwest::get(url.as_str())
                .await
                .ok()?
                .error_for_status()
                .ok()?;
            resp.text().await.ok()
        }
        other => {
            log::error!("unsupported protocol {other}:");
            None
        }
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// user scripts must live in <config dir>/user-scripts and end with .js
pub fn load_user_scripts(config: &crate::config::Config) -> Vec<String> {
    let user_scripts = normalize(&config_dir().join("user-scripts"));
    let mut out = Vec::new();
    for entry in config.str_list("scripts").unwrap_or_default() {
        let entry = entry.replace("${userData}", &config_dir().to_string_lossy());
        let path = normalize(&std::env::current_dir().unwrap_or_default().join(&entry));
        log::info!("load user script {}", path.display());
        if !path.starts_with(&user_scripts) {
            log::error!(
                "script must be in {}: {}",
                user_scripts.display(),
                path.display()
            );
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("js") {
            log::error!("script must end with .js: {}", path.display());
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(s) if !s.is_empty() => out.push(s),
            Ok(_) => {}
            Err(e) => log::error!("{}: {e}", path.display()),
        }
    }
    out
}

pub fn dir_size(path: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

pub fn format_size(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", units[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whatsapp_send_url_is_converted_to_web_url() {
        let url = convert_whatsapp_url("whatsapp://send/?phone=5511999999999&text=hello").unwrap();
        assert_eq!(
            url.as_str(),
            "https://web.whatsapp.com/send/?phone=5511999999999&text=hello"
        );
    }

    #[test]
    fn other_schemes_are_not_converted() {
        assert!(convert_whatsapp_url("https://web.whatsapp.com/").is_none());
        assert!(convert_whatsapp_url("not a url").is_none());
    }

    #[test]
    fn unread_count_is_read_from_favicon_url() {
        assert_eq!(
            unread_from_favicon("https://web.whatsapp.com/favicon/1x/f12/a.webp"),
            12
        );
        assert_eq!(
            unread_from_favicon("https://web.whatsapp.com/favicon/1x/favicon.webp"),
            0
        );
        assert_eq!(unread_from_favicon("https://example.com/favicon.ico"), 0);
    }

    #[test]
    fn sizes_are_formatted_with_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MiB");
    }

    #[test]
    fn dir_size_includes_nested_files() {
        let base = std::env::temp_dir().join(format!("elecwhat-dir-size-{}", std::process::id()));
        let nested = base.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("file"), b"12345").unwrap();

        assert_eq!(dir_size(&base), 5);
        std::fs::remove_dir_all(base).unwrap();
    }
}
