mod autostart;
mod config;
mod dbus;
mod page;
mod util;

use config::{Config, StateFile};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{
    AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent,
    image::Image,
    menu::{AboutMetadataBuilder, CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    webview::{
        DownloadEvent, NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse,
    },
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;
use util::{Translations, tr};

const PKG_NAME: &str = "elecwhat";
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36";

pub struct AppState {
    pub config: Config,
    pub persist: StateFile,
    pub translations: Translations,
    pub user_agent: String,
    pub icons_dir: PathBuf,
    pub url: Url,
    pub debug: bool,
    pub show_at_startup: bool,
    pub quit_on_close: AtomicBool,
    pub quitting: AtomicBool,
    pub page_loaded: AtomicBool,
    pub zoom: Mutex<f64>,
    pub newest_icon: Mutex<Option<String>>,
    pub icon_cache: Mutex<HashMap<String, Image<'static>>>,
    pub user_icon_cache: Mutex<HashMap<String, Option<Image<'static>>>>,
    pub dbus: OnceLock<dbus::Dbus>,
}

// ---- commands (the only things web.whatsapp.com may call, see capabilities/remote.json) ----

#[tauri::command]
fn window_toggle(app: AppHandle) {
    util::window_toggle(&app);
}

#[tauri::command]
fn menu_toggle(app: AppHandle) {
    if let Some(w) = util::main_window(&app) {
        if w.is_menu_visible().unwrap_or(false) {
            let _ = w.hide_menu();
        } else {
            let _ = w.show_menu();
        }
    }
}

/// Links are handled natively by `on_navigation` / `on_new_window`; the URL is
/// untrusted page input, so only web-ish schemes are handed to the system.
fn open_external(app: &AppHandle, url: &Url) {
    if !matches!(url.scheme(), "http" | "https" | "mailto" | "tel") {
        log::warn!("refusing to open {url}");
        return;
    }
    if let Err(e) = app.opener().open_url(url.as_str(), None::<&str>) {
        log::error!("open {url}: {e}");
    }
}

/// Keeps downloads in the system download dir with a safe, non-colliding name.
fn download_path(dir: &Path, name: &str) -> PathBuf {
    let safe = Path::new(name)
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("download"));
    let mut path = dir.join(&safe);
    let mut n = 1;
    while path.exists() {
        let stem = safe.file_stem().unwrap_or_default().to_string_lossy();
        let ext = safe
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        path = dir.join(format!("{stem}-{n}{ext}"));
        n += 1;
    }
    path
}

fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// WebKitGTK has no desktop-notification bridge of its own: the page script
/// replaces window.Notification and sends it here; a click is sent back to the page.
#[tauri::command]
fn notify(app: AppHandle, id: u64, title: String, body: String) {
    std::thread::spawn(move || {
        let res = notify_rust::Notification::new()
            .appname("elecwhat")
            .summary(&title)
            .body(&escape_markup(&body))
            .icon("elecwhat")
            .hint(notify_rust::Hint::DesktopEntry("elecwhat".into()))
            .action("default", "Open")
            .show();
        match res {
            Ok(handle) => handle.wait_for_action(|action| {
                if action == "default" {
                    util::window_show(&app);
                    if let Some(w) = util::main_window(&app) {
                        let _ = w.eval(format!(
                            "window.__ewNotifClick && window.__ewNotifClick({id})"
                        ));
                    }
                }
            }),
            Err(e) => log::error!("notification: {e}"),
        }
    });
}

#[tauri::command]
async fn favicon_updated(app: AppHandle, url: String) {
    let state = app.state::<AppState>();
    *state.newest_icon.lock().unwrap() = Some(url.clone());
    let Some(img) = util::get_icon(&state, &url).await else {
        return;
    };
    // sometimes an old icon takes time to download and arrives late
    if state.newest_icon.lock().unwrap().as_deref() != Some(url.as_str()) {
        return;
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(img));
    }
    if let Some(d) = state.dbus.get() {
        d.set_badge_count(util::unread_from_favicon(&url)).await;
    }
}

// ---- second instance / whatsapp: urls -----------------------------------------------

fn handle_whatsapp_url(app: &AppHandle, url: &str) {
    if let Some(web) = util::convert_whatsapp_url(url) {
        log::info!("Loading WhatsApp URL: {web}");
        util::window_show(app);
        if let Some(w) = util::main_window(app) {
            let _ = w.navigate(web);
        }
    }
}

fn on_second_instance(app: &AppHandle, argv: &[String]) {
    log::debug!("second-instance {argv:?}");
    if let Some(u) = argv
        .iter()
        .find(|a| a.starts_with(&format!("{}:", util::URL_SCHEME)))
    {
        handle_whatsapp_url(app, u);
    } else if argv.iter().any(|a| a == "--hide") {
        util::window_hide(app);
    } else if argv.iter().any(|a| a == "--toggle") {
        util::window_toggle(app);
    } else if argv.iter().any(|a| a == "--quit") {
        app.exit(0);
    } else {
        util::window_show(app); // --show
    }
}

// ---- webkit (linux) -----------------------------------------------------------------

fn configure_webkit(window: &tauri::WebviewWindow, spellcheck: bool, langs: Vec<String>) {
    let _ = window.with_webview(move |wv| {
        use webkit2gtk::{SettingsExt, WebContextExt, WebViewExt};
        let view = wv.inner();
        // wry does not turn these on: needed for calls / voice+video messages
        if let Some(s) = view.settings() {
            s.set_enable_media_stream(true);
            s.set_enable_webrtc(true);
            s.set_enable_mediasource(true);
            s.set_media_playback_requires_user_gesture(false);
        }
        if let Some(ctx) = view.context() {
            ctx.set_spell_checking_enabled(spellcheck);
            let langs: Vec<String> = langs.iter().map(|l| l.replace('-', "_")).collect();
            let refs: Vec<&str> = langs.iter().map(String::as_str).collect();
            ctx.set_spell_checking_languages(&refs);
        }
    });
}

fn clear_cache(app: &AppHandle) {
    util::window_show(app);
    let Some(w) = util::main_window(app) else {
        return;
    };
    let app = app.clone();
    let _ = w.with_webview(move |wv| {
        use webkit2gtk::{WebViewExt, WebsiteDataManagerExt};
        let Some(dm) = wv.inner().website_data_manager() else { return };
        let dir = dm.base_cache_directory().map(|d| PathBuf::from(d.as_str()));
        let size_before = dir.as_deref().map(util::dir_size).unwrap_or(0);
        let state = app.state::<AppState>();
        let t = &state.translations;
        let message = format!(
            "{}\n\n{}",
            tr(t, "clear_cache_message", "Clear the cache ({size})?").replace("{size}", &util::format_size(size_before)),
            tr(
                t,
                "clear_cache_detail",
                "Cached images and videos will be downloaded again when needed. You will stay logged in and your chats are not affected."
            )
        );
        let ok = tr(t, "clear_cache_ok", "Clear cache");
        let cancel = tr(t, "cancel", "Cancel");
        let done_tpl = tr(t, "clear_cache_done", "Cache cleared, {size} freed");
        let app2 = app.clone();
        app.dialog()
            .message(message)
            .title(tr(t, "clear_cache", "Clear cache"))
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom(ok, cancel))
            .show(move |confirmed| {
                if !confirmed {
                    return;
                }
                let Some(w) = util::main_window(&app2) else { return };
                let _ = w.with_webview(move |wv| {
                    use webkit2gtk::{glib, WebsiteDataManagerExtManual, WebsiteDataTypes};
                    let Some(dm) = wv.inner().website_data_manager() else { return };
                    // HTTP caches only: cookies, local storage and IndexedDB (= login + chats) are kept
                    let types = WebsiteDataTypes::DISK_CACHE | WebsiteDataTypes::MEMORY_CACHE;
                    let app3 = app2.clone();
                    dm.clear(types, glib::TimeSpan::from_seconds(0), None::<&webkit2gtk::gio::Cancellable>, move |res| {
                        if let Err(e) = res {
                            log::error!("clear cache: {e}");
                        }
                        let after = dir.as_deref().map(util::dir_size).unwrap_or(0);
                        let freed = util::format_size(size_before.saturating_sub(after));
                        let _ = notify_rust::Notification::new()
                            .appname("elecwhat")
                            .summary(PKG_NAME)
                            .body(&done_tpl.replace("{size}", &freed))
                            .show();
                        if let Some(w) = util::main_window(&app3) {
                            let _ = w.eval("location.reload()");
                        }
                    });
                });
            });
    });
}

// ---- window / tray ------------------------------------------------------------------

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &MenuItem::with_id(app, "reload", "Reload", true, Some("CmdOrCtrl+R"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "zoom_in", "Zoom In", true, Some("CmdOrCtrl+="))?,
            &MenuItem::with_id(app, "zoom_out", "Zoom Out", true, Some("CmdOrCtrl+-"))?,
            &MenuItem::with_id(app, "zoom_reset", "Actual Size", true, Some("CmdOrCtrl+0"))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "devtools",
                "Toggle Developer Tools",
                true,
                Some("CmdOrCtrl+Shift+I"),
            )?,
        ],
    )?;
    let help = Submenu::with_items(app, "Help", true, &[&about_item(app)?])?;
    Menu::with_items(app, &[&view, &help])
}

fn about_item(app: &AppHandle) -> tauri::Result<PredefinedMenuItem<tauri::Wry>> {
    let t = &app.state::<AppState>().translations;
    let meta = AboutMetadataBuilder::new()
        .name(Some("ElecWhat"))
        .version(Some(app.package_info().version.to_string()))
        .authors(Some(vec!["Pierre Carru".to_string()]))
        .website(Some("https://github.com/piec/elecwhat"))
        .license(Some("GPL-3.0-or-later"))
        .icon(app.default_window_icon().cloned())
        .build();
    PredefinedMenuItem::about(app, Some(&tr(t, "about", "About")), Some(meta))
}

fn on_window_menu_event(window: &tauri::Window, id: &str) {
    let app = window.app_handle();
    let Some(w) = util::main_window(app) else {
        return;
    };
    let state = app.state::<AppState>();
    let mut zoom = state.zoom.lock().unwrap();
    match id {
        "reload" => {
            let _ = w.eval("location.reload()");
        }
        "zoom_in" => *zoom = (*zoom + 0.1).min(3.0),
        "zoom_out" => *zoom = (*zoom - 0.1).max(0.3),
        "zoom_reset" => *zoom = 1.0,
        "devtools" => {
            if w.is_devtools_open() {
                w.close_devtools();
            } else {
                w.open_devtools();
            }
        }
        _ => {}
    }
    if id.starts_with("zoom_") {
        let _ = w.set_zoom(*zoom);
    }
}

fn create_window(app: &AppHandle, initial_url: Url) -> Result<(), Box<dyn std::error::Error>> {
    let state = app.state::<AppState>();
    let config = &state.config;
    let page_host = state.url.host_str().map(String::from);
    let permission_host = page_host.clone();
    let nav_host = page_host.clone();
    let new_host = page_host.clone();
    let page_app = app.clone();
    let nav_app = app.clone();
    let new_app = app.clone();
    let download_app = app.clone();

    let shim = page::script(
        page_host.as_deref().unwrap_or("web.whatsapp.com"),
        &config.str("notification-prefix", ""),
        config.bool("esc-toggle-window", false),
        config.bool("menu-bar", true) && config.bool("menu-bar-auto-hide", true),
    );

    let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(initial_url))
        .title("ElecWhat")
        .inner_size(1099.0, 800.0)
        .visible(state.show_at_startup)
        .user_agent(&state.user_agent)
        .initialization_script(shim)
        .disable_drag_drop_handler()
        .on_navigation(move |url| {
            if url.scheme() == "https"
                && url.host_str().map(String::from).as_deref() == nav_host.as_deref()
            {
                return true;
            }
            open_external(&nav_app, url);
            false
        })
        .on_new_window(move |url, _| {
            if url.scheme() == "https"
                && url.host_str().map(String::from).as_deref() == new_host.as_deref()
            {
                NewWindowResponse::Allow
            } else {
                open_external(&new_app, &url);
                NewWindowResponse::Deny
            }
        })
        .on_download(move |_webview, event| {
            if let DownloadEvent::Requested { destination, .. } = event {
                let name = destination
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("download");
                let dir = download_app
                    .path()
                    .download_dir()
                    .unwrap_or_else(|_| std::env::temp_dir());
                *destination = download_path(&dir, name);
            }
            true
        })
        .on_permission_request(move |webview, kind| {
            // Only the configured page may be granted camera/mic/screen access.
            let same_host = webview
                .url()
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .as_deref()
                == permission_host.as_deref();
            if !same_host {
                log::warn!("denying {kind:?} from an unexpected origin");
                return PermissionResponse::Deny;
            }
            match kind {
                PermissionKind::Microphone
                | PermissionKind::Camera
                | PermissionKind::DisplayCapture
                | PermissionKind::Notifications => PermissionResponse::Allow,
                _ => PermissionResponse::Default,
            }
        })
        .on_page_load(move |w, payload| {
            let st = page_app.state::<AppState>();
            match payload.event() {
                PageLoadEvent::Started => st.page_loaded.store(false, Ordering::SeqCst),
                PageLoadEvent::Finished => {
                    if payload.url().host_str().map(String::from) != page_host {
                        return;
                    }
                    st.page_loaded.store(true, Ordering::SeqCst);
                    log::debug!("page load finished");
                    for script in util::load_user_scripts(&st.config) {
                        let _ = w.eval(script);
                    }
                }
            }
        });

    if config.bool("menu-bar", true) {
        builder = builder
            .menu(build_menu(app)?)
            .on_menu_event(|window, event| {
                on_window_menu_event(window, event.id().as_ref());
            });
    }

    let window = builder.build()?;
    if config.bool("menu-bar", true) && config.bool("menu-bar-auto-hide", true) {
        let _ = window.hide_menu();
    }
    if state.debug || config.bool("open-dev-tools", false) {
        window.open_devtools();
    }

    let langs = config
        .str_list("spellcheck-languages")
        .unwrap_or_else(util::preferred_langs);
    log::debug!("spellLangs {langs:?}");
    configure_webkit(&window, config.bool("spellcheck", true), langs);
    Ok(())
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let state = app.state::<AppState>();
    let t = &state.translations;

    let quit_on_close = CheckMenuItem::with_id(
        app,
        "quit_on_close",
        tr(t, "quit_on_close", "Quit on close"),
        true,
        state.quit_on_close.load(Ordering::SeqCst),
        None::<&str>,
    )?;
    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        tr(t, "autostart", "Start on login"),
        autostart::supported(),
        autostart::is_enabled(),
        None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(
                app,
                "show_hide",
                tr(t, "show_hide", "Show/Hide"),
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(app, "reload", tr(t, "reload", "Reload"), true, None::<&str>)?,
            &MenuItem::with_id(
                app,
                "clear_cache",
                tr(t, "clear_cache_menu", "Clear cache…"),
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &quit_on_close,
            &autostart_item,
            &PredefinedMenuItem::separator(app)?,
            &about_item(app)?,
            &MenuItem::with_id(app, "quit", tr(t, "quit", "Quit"), true, None::<&str>)?,
        ],
    )?;

    let icon = util::user_icon(&state, "app").unwrap_or_else(|| {
        Image::from_bytes(include_bytes!("../../static/app.png"))
            .unwrap_or_else(|_| app.default_window_icon().unwrap().clone())
    });

    // NB: on Linux tray *click* events are not delivered by the platform (AppIndicator
    // only has a menu), hence the explicit "Show/Hide" entry.
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip(PKG_NAME)
        .menu(&menu)
        .on_menu_event(move |app, event| {
            let state = app.state::<AppState>();
            match event.id().as_ref() {
                "show_hide" => {
                    util::window_toggle(app);
                }
                "reload" => {
                    if let Some(w) = util::main_window(app) {
                        let _ = w.eval("location.reload()");
                    }
                }
                "clear_cache" => clear_cache(app),
                "quit_on_close" => {
                    let checked = quit_on_close.is_checked().unwrap_or(false);
                    state.quit_on_close.store(checked, Ordering::SeqCst);
                    state.persist.set("quit-on-close", json!(checked));
                }
                "autostart" => {
                    autostart::set(autostart_item.is_checked().unwrap_or(false));
                    let _ = autostart_item.set_checked(autostart::is_enabled());
                }
                "quit" => {
                    state.quitting.store(true, Ordering::SeqCst);
                    app.exit(0);
                }
                _ => {}
            }
        })
        .build(app)?;
    Ok(())
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let state = handle.state::<AppState>();
    log::info!("config file {}", state.config.file.display());

    if let Some(err) = &state.config.parse_error {
        let t = &state.translations;
        let h = handle.clone();
        handle
            .dialog()
            .message(format!("{}:\n\n{err}", state.config.file.display()))
            .title(tr(t, "config_title", "Configuration file"))
            .kind(MessageDialogKind::Error)
            .buttons(MessageDialogButtons::OkCancelCustom(
                tr(t, "config_continue", "Continue with default config"),
                tr(t, "config_quit", "Quit"),
            ))
            .show(move |cont| {
                if !cont {
                    h.exit(0);
                }
            });
    }

    if state.config.bool("register-url-scheme", true) && !cfg!(debug_assertions) {
        let dl = handle.deep_link();
        if !dl.is_registered(util::URL_SCHEME).unwrap_or(false) {
            log::info!(
                "Registering as default protocol client for {}://",
                util::URL_SCHEME
            );
            if let Err(e) = dl.register(util::URL_SCHEME) {
                log::warn!("register url scheme: {e}");
            }
        }
    }

    // launched with a whatsapp: url -> open it directly
    let initial = std::env::args()
        .find(|a| a.starts_with(&format!("{}:", util::URL_SCHEME)))
        .and_then(|a| util::convert_whatsapp_url(&a))
        .unwrap_or_else(|| state.url.clone());
    create_window(&handle, initial)?;

    if state.config.bool("refresh-autostart", true) {
        autostart::refresh();
    }
    build_tray(&handle)?;

    if state.config.bool("dbus", true) {
        let h = handle.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(d) = dbus::Dbus::start(h.clone()).await {
                let _ = h.state::<AppState>().dbus.set(d);
            }
        });
    }

    // user css: applied as a WebKit user style sheet (not subject to the page's CSP)
    let css_entries = state.config.str_list("css").unwrap_or_default();
    if !css_entries.is_empty() {
        let h = handle.clone();
        tauri::async_runtime::spawn(async move {
            let mut css = String::new();
            for entry in css_entries {
                match util::load_css_entry(&entry).await {
                    Some(c) => {
                        css.push_str(&c);
                        css.push('\n');
                    }
                    None => log::error!("error inserting {entry}"),
                }
            }
            if let Some(w) = util::main_window(&h) {
                let _ = w.with_webview(move |wv| {
                    use webkit2gtk::{
                        UserContentInjectedFrames, UserContentManagerExt, UserStyleLevel,
                        UserStyleSheet, WebViewExt,
                    };
                    if let Some(ucm) = wv.inner().user_content_manager() {
                        ucm.add_style_sheet(&UserStyleSheet::new(
                            &css,
                            UserContentInjectedFrames::TopFrame,
                            UserStyleLevel::User,
                            &[],
                            &[],
                        ));
                    }
                });
            }
        });
    }

    // WebKit has no "did-fail-load" hook: on_page_load tells us when the page is
    // up, if it never does (offline at startup, ...) retry like elecwhat always did.
    // A misconfigured 0 must not turn the watchdog into a busy loop.
    let interval = Duration::from_millis(state.config.u64("retry-interval", 15000).max(1000));
    let h = handle.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(interval);
            let st = h.state::<AppState>();
            if !st.page_loaded.load(Ordering::SeqCst) {
                log::warn!("page not loaded, retry");
                let _ = notify_rust::Notification::new()
                    .appname("elecwhat")
                    .summary(PKG_NAME)
                    .body("Failed to load")
                    .show();
                if let Some(w) = util::main_window(&h) {
                    let _ = w.navigate(st.url.clone());
                }
            }
        }
    });
    Ok(())
}

pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let config_dir = util::config_dir();
    let _ = std::fs::create_dir_all(&config_dir);
    let config = Config::load(config_dir.join("config.json"));

    // log level: CLI option takes priority over config option
    let level_str = args
        .iter()
        .find_map(|a| a.strip_prefix("--log-level="))
        .map(String::from)
        .unwrap_or_else(|| config.str("log-level", "info"));
    let level = match level_str.as_str() {
        "silent" => log::LevelFilter::Off,
        "fatal" | "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "debug" => log::LevelFilter::Debug,
        "trace" | "verbose" => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Warn)
        .filter_module("elecwhat_lib", level)
        .init();

    let debug = std::env::var("DEBUG").as_deref() == Ok("1");
    let persist = StateFile::load(config_dir.join("persistent-state.json"));
    let quit_on_close = persist
        .get_bool("quit-on-close")
        .unwrap_or_else(|| config.bool("quit-on-close", util::is_gnome()));
    let lang = util::preferred_langs().remove(0);
    let state = AppState {
        translations: util::load_translations(&lang),
        user_agent: config.str("user-agent", DEFAULT_USER_AGENT),
        icons_dir: config
            .get("icons-directory")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| config_dir.join("user-icons")),
        url: config
            .str("url", "https://web.whatsapp.com/")
            .parse()
            .unwrap_or_else(|_| {
                log::error!("invalid `url` in config, falling back to https://web.whatsapp.com/");
                Url::parse("https://web.whatsapp.com/").expect("valid fallback URL")
            }),
        debug,
        show_at_startup: quit_on_close || debug || config.bool("show-at-startup", true),
        quit_on_close: AtomicBool::new(quit_on_close),
        quitting: AtomicBool::new(false),
        page_loaded: AtomicBool::new(false),
        zoom: Mutex::new(1.0),
        newest_icon: Mutex::new(None),
        icon_cache: Mutex::new(HashMap::new()),
        user_icon_cache: Mutex::new(HashMap::new()),
        dbus: OnceLock::new(),
        persist,
        config,
    };

    tauri::Builder::default()
        // must be the first plugin
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            on_second_instance(app, &argv)
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // not VISIBLE: `show-at-startup` decides
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            notify,
            window_toggle,
            favicon_updated,
            menu_toggle
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let st = app.state::<AppState>();
                if !st.quit_on_close.load(Ordering::SeqCst) && !st.quitting.load(Ordering::SeqCst) {
                    api.prevent_close();
                    util::window_hide(app); // minimize to tray
                }
            }
        })
        .setup(setup)
        .run(tauri::generate_context!())
        .expect("error while running elecwhat");
}
