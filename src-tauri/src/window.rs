use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{
    App, AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::cli::{self, Action, Cli};
use crate::state::AppState;

pub const LABEL: &str = "main";

const DEFAULT_URL: &str = "https://web.whatsapp.com/";
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36";

pub fn create(app: &App) -> Result<()> {
    let state = app.state::<AppState>().inner().clone();
    let (url, user_agent, open_devtools) = {
        let config = state.config();
        (
            config.get_string("url", DEFAULT_URL),
            config.get_string("user-agent", DEFAULT_USER_AGENT),
            cli::is_debug() || config.get_bool("open-dev-tools", false),
        )
    };

    let bounds = state.initial_bounds();
    let url = tauri::Url::parse(&url).with_context(|| format!("invalid url in config: {url}"))?;

    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .title("ElecWhat")
        .inner_size(bounds.width, bounds.height)
        .visible(!state.start_hidden())
        .user_agent(&user_agent)
        .data_directory(state.paths().webview_data.clone());

    if let (Some(x), Some(y)) = (bounds.x, bounds.y) {
        builder = builder.position(x, y);
    }

    let window = builder.build().context("failed to create main window")?;
    if open_devtools {
        window.open_devtools();
    }

    install_handlers(&state, &window);
    Ok(())
}

fn install_handlers(state: &AppState, window: &WebviewWindow) {
    let state = state.clone();
    let window_handle = window.clone();

    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            state.save_bounds(&window_handle);
            if !state.is_quitting() && !state.quit_on_close() {
                api.prevent_close();
                if let Err(err) = window_handle.hide() {
                    tracing::error!("failed to hide window: {err}");
                }
            }
        }
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            state.maybe_save_bounds(&window_handle);
        }
        WindowEvent::Destroyed => state.save_bounds(&window_handle),
        _ => {}
    });
}

/// Shows a second instance launched with `--hide`, `--toggle`, `--quit`, ...
pub fn handle_second_instance(app: &AppHandle, cli: Cli) {
    let state = app.state::<AppState>().inner().clone();

    if cli.action == Some(Action::Quit) {
        state.set_quitting();
        app.exit(0);
        return;
    }

    let Some(window) = app.get_webview_window(LABEL) else {
        tracing::warn!("second instance: main window not found");
        return;
    };

    match cli.action {
        Some(Action::Quit) => unreachable!("handled above"),
        Some(Action::Hide) => hide_window(&window),
        Some(Action::Toggle) => toggle_window(&window),
        Some(Action::Show) | None => show_window(&window),
    }
}

pub fn maybe_show_config_error(app: &App) {
    let state = app.state::<AppState>().inner().clone();
    let Some(error) = state.config_parse_error() else {
        return;
    };

    tracing::warn!(
        "config parse error in {}: {error}",
        state.paths().config_file.display()
    );
    let path = state.paths().config_file.display().to_string();
    let handle = app.handle().clone();

    app.dialog()
        .message(format!("{path}:\n\n{error}"))
        .title("Configuration file")
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Continue with default config".to_string(),
            "Quit".to_string(),
        ))
        .show(move |ok| {
            if !ok {
                handle.exit(0);
            }
        });
}

fn show_window(window: &WebviewWindow) {
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    if let Err(err) = window.show() {
        tracing::error!("failed to show window: {err}");
    }
    if let Err(err) = window.set_focus() {
        tracing::error!("failed to focus window: {err}");
    }
}

fn hide_window(window: &WebviewWindow) {
    if let Err(err) = window.hide() {
        tracing::error!("failed to hide window: {err}");
    }
}

fn toggle_window(window: &WebviewWindow) {
    if window.is_visible().unwrap_or(false) {
        hide_window(window);
    } else {
        show_window(window);
    }
}

/// Window bounds persisted in `persistent-state.json`, in logical pixels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Bounds {
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default = "default_width")]
    pub width: f64,
    #[serde(default = "default_height")]
    pub height: f64,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: default_width(),
            height: default_height(),
        }
    }
}

fn default_width() -> f64 {
    1099.0
}

fn default_height() -> f64 {
    800.0
}

pub fn sanitize_bounds(mut bounds: Bounds) -> Bounds {
    if !bounds.width.is_finite() || bounds.width < 200.0 {
        bounds.width = default_width();
    }
    if !bounds.height.is_finite() || bounds.height < 200.0 {
        bounds.height = default_height();
    }
    if bounds.x.is_some_and(|x| !x.is_finite()) {
        bounds.x = None;
    }
    if bounds.y.is_some_and(|y| !y.is_finite()) {
        bounds.y = None;
    }
    bounds
}

pub fn current_bounds(window: &WebviewWindow) -> Option<Bounds> {
    if window.is_minimized().unwrap_or(false) {
        return None;
    }

    let scale = window.scale_factor().ok()?;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }

    let position = window.outer_position().ok()?;
    let size = window.inner_size().ok()?;

    Some(Bounds {
        x: Some((position.x as f64 / scale).round()),
        y: Some((position.y as f64 / scale).round()),
        width: (size.width as f64 / scale).round(),
        height: (size.height as f64 / scale).round(),
    })
}

pub fn version_info() -> String {
    let mut details = vec![format!("tauri {}", tauri::VERSION)];
    if let Some(session) = cli::session_type() {
        details.push(session);
    }
    details.push(cli::packaging().to_string());
    format!("{} ({})", env!("CARGO_PKG_VERSION"), details.join(", "))
}
