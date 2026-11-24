//! Command line arguments, mirroring the flags the Electron version handled.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Show,
    Hide,
    Toggle,
    Quit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cli {
    pub action: Option<Action>,
    pub log_level: Option<String>,
}

impl Cli {
    pub fn parse<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut cli = Self::default();
        let mut args = args.into_iter().map(|arg| arg.as_ref().to_string());

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--show" => cli.action = Some(Action::Show),
                "--hide" => cli.action = Some(Action::Hide),
                "--toggle" => cli.action = Some(Action::Toggle),
                "--quit" => cli.action = Some(Action::Quit),
                "--log-level" => {
                    if let Some(level) = args.next() {
                        cli.log_level = Some(level);
                    }
                }
                _ => {
                    if let Some(level) = arg.strip_prefix("--log-level=") {
                        cli.log_level = Some(level.to_string());
                    }
                }
            }
        }

        cli
    }
}

pub fn is_debug() -> bool {
    matches!(std::env::var("DEBUG").as_deref(), Ok("1"))
}

/// GNOME Shell doesn't show a tray icon without a (rarely installed) extension,
/// so hiding to tray on close would leave the app with no way to reopen it.
pub fn default_quit_on_close() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase()
        .contains("gnome")
}

/// How the app was installed, as far as we can tell from the environment.
pub fn packaging() -> &'static str {
    if std::env::var_os("SNAP").is_some() {
        "snap"
    } else if std::env::var_os("FLATPAK_ID").is_some() {
        "flatpak"
    } else if std::env::var_os("APPIMAGE").is_some() {
        "appimage"
    } else if cfg!(debug_assertions) {
        "dev"
    } else {
        "packaged"
    }
}

pub fn session_type() -> Option<String> {
    std::env::var("XDG_SESSION_TYPE")
        .ok()
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags_in_order() {
        let cli = Cli::parse(["elecwhat", "--hide", "--show"]);
        assert_eq!(cli.action, Some(Action::Show));
    }

    #[test]
    fn parses_log_level() {
        let cli = Cli::parse(["elecwhat", "--log-level=debug"]);
        assert_eq!(cli.log_level.as_deref(), Some("debug"));

        let cli = Cli::parse(["elecwhat", "--log-level", "trace"]);
        assert_eq!(cli.log_level.as_deref(), Some("trace"));
    }

    #[test]
    fn ignores_unknown_arguments() {
        let cli = Cli::parse(["elecwhat", "--whatever", "whatsapp://send"]);
        assert_eq!(cli, Cli::default());
    }
}
