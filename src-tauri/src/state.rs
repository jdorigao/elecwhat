use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::WebviewWindow;

use crate::cli::{self, Action, Cli};
use crate::config::JsonConfig;
use crate::paths::Paths;
use crate::window::{self, Bounds};

struct Inner {
    paths: Paths,
    config: Mutex<JsonConfig>,
    persist: Mutex<JsonConfig>,
    quitting: AtomicBool,
    quit_on_close: AtomicBool,
    quit_on_start: bool,
    start_hidden: bool,
    last_bounds_save: Mutex<Instant>,
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl AppState {
    pub fn new(paths: Paths, config: JsonConfig, cli: &Cli) -> Self {
        let quit_on_close = config.get_bool("quit-on-close", cli::default_quit_on_close());

        let mut show_at_startup = cli::is_debug() || config.get_bool("show-at-startup", true);
        if quit_on_close {
            show_at_startup = true;
        }
        match cli.action {
            Some(Action::Show) | Some(Action::Toggle) => show_at_startup = true,
            Some(Action::Hide) => show_at_startup = false,
            Some(Action::Quit) | None => {}
        }

        let persist = JsonConfig::load(&paths.state_file);
        let quit_on_start = cli.action == Some(Action::Quit);

        Self(Arc::new(Inner {
            paths,
            config: Mutex::new(config),
            persist: Mutex::new(persist),
            quitting: AtomicBool::new(false),
            quit_on_close: AtomicBool::new(quit_on_close),
            quit_on_start,
            start_hidden: !show_at_startup,
            last_bounds_save: Mutex::new(Instant::now()),
        }))
    }

    pub fn paths(&self) -> &Paths {
        &self.0.paths
    }

    pub fn config(&self) -> MutexGuard<'_, JsonConfig> {
        self.0
            .config
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn persist(&self) -> MutexGuard<'_, JsonConfig> {
        self.0
            .persist
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn config_parse_error(&self) -> Option<String> {
        self.config().parse_error().map(str::to_owned)
    }

    pub fn start_hidden(&self) -> bool {
        self.0.start_hidden
    }

    pub fn quit_on_start(&self) -> bool {
        self.0.quit_on_start
    }

    pub fn is_quitting(&self) -> bool {
        self.0.quitting.load(Ordering::Relaxed)
    }

    pub fn set_quitting(&self) {
        self.0.quitting.store(true, Ordering::Relaxed);
    }

    pub fn quit_on_close(&self) -> bool {
        self.0.quit_on_close.load(Ordering::Relaxed)
    }

    /// Used by the tray menu once it exists.
    #[allow(dead_code)]
    pub fn set_quit_on_close(&self, value: bool) {
        self.0.quit_on_close.store(value, Ordering::Relaxed);
        if let Err(err) = self.config().set("quit-on-close", value) {
            tracing::error!("failed to save quit-on-close: {err:#}");
        }
    }

    pub fn initial_bounds(&self) -> Bounds {
        let bounds: Bounds = self.persist().get("window-bounds", Bounds::default());
        let mut bounds = window::sanitize_bounds(bounds);
        if cli::is_debug() {
            bounds.width += 1000.0;
        }
        bounds
    }

    pub fn save_bounds(&self, window: &WebviewWindow) {
        let Some(bounds) = window::current_bounds(window) else {
            return;
        };
        if let Err(err) = self.persist().set("window-bounds", bounds) {
            tracing::error!("failed to save window bounds: {err:#}");
        }
    }

    /// Called on every move/resize event; at most one write per second.
    pub fn maybe_save_bounds(&self, window: &WebviewWindow) {
        let mut last = self
            .0
            .last_bounds_save
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.elapsed() < Duration::from_secs(1) {
            return;
        }
        *last = Instant::now();
        drop(last);
        self.save_bounds(window);
    }
}
