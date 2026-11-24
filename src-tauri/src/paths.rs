use std::path::PathBuf;

/// Paths used by the app. They intentionally match the Electron version so an
/// existing installation keeps its config, scripts and icons.
#[derive(Debug, Clone)]
#[allow(dead_code)] // scripts/icons directories are used by later migration phases
pub struct Paths {
    pub user_data: PathBuf,
    pub config_file: PathBuf,
    pub state_file: PathBuf,
    pub scripts_dir: PathBuf,
    pub icons_dir: PathBuf,
    pub webview_data: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let user_data = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("elecwhat");

        Self {
            config_file: user_data.join("config.json"),
            state_file: user_data.join("persistent-state.json"),
            scripts_dir: user_data.join("user-scripts"),
            icons_dir: user_data.join("user-icons"),
            webview_data: user_data.join("webkit"),
            user_data,
        }
    }
}
