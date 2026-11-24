//! ~/.config/elecwhat/config.json (JSONC, read-only) and persistent-state.json (read/write).

use jsonc_parser::{ParseOptions, parse_to_serde_value};
use serde_json::{Map, Value};
use std::{fs, path::PathBuf, sync::Mutex};

pub struct Config {
    pub file: PathBuf,
    data: Map<String, Value>,
    pub parse_error: Option<String>,
}

impl Config {
    pub fn load(file: PathBuf) -> Self {
        let mut cfg = Config {
            file,
            data: Map::new(),
            parse_error: None,
        };
        let Ok(text) = fs::read_to_string(&cfg.file) else {
            return cfg;
        };
        match parse_to_serde_value(&text, &ParseOptions::default()) {
            Ok(Some(Value::Object(map))) => cfg.data = map,
            Ok(_) => {}
            Err(e) => cfg.parse_error = Some(e.to_string()),
        }
        cfg
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(default)
    }

    pub fn str(&self, key: &str, default: &str) -> String {
        self.get(key)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    }

    pub fn u64(&self, key: &str, default: u64) -> u64 {
        self.get(key).and_then(Value::as_u64).unwrap_or(default)
    }

    pub fn str_list(&self, key: &str) -> Option<Vec<String>> {
        self.get(key).and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
    }
}

/// Small plain-JSON file for values the app itself changes at runtime (tray toggles).
/// Kept separate from config.json so the user's comments/formatting are never rewritten.
pub struct StateFile {
    file: PathBuf,
    data: Mutex<Map<String, Value>>,
}

impl StateFile {
    pub fn load(file: PathBuf) -> Self {
        let data = fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<Map<String, Value>>(&t).ok())
            .unwrap_or_default();
        StateFile {
            file,
            data: Mutex::new(data),
        }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.data.lock().unwrap().get(key).and_then(Value::as_bool)
    }

    pub fn set(&self, key: &str, value: Value) {
        let mut data = self.data.lock().unwrap();
        data.insert(key.to_string(), value);
        if let Ok(text) = serde_json::to_string_pretty(&*data)
            && let Err(e) = fs::write(&self.file, text)
        {
            log::error!("cannot save {}: {e}", self.file.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn temp_file(ext: &str) -> PathBuf {
        let n = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("elecwhat-test-{}-{n}.{ext}", std::process::id()))
    }

    #[test]
    fn loads_jsonc_with_comments_and_trailing_commas() {
        let file = temp_file("json");
        fs::write(
            &file,
            r#"{
                // a comment
                "spellcheck": true,
                "spellcheck-languages": ["en-US",],
            }"#,
        )
        .unwrap();

        let cfg = Config::load(file.clone());
        assert!(cfg.parse_error.is_none());
        assert!(cfg.bool("spellcheck", false));
        assert_eq!(cfg.str_list("spellcheck-languages").unwrap(), vec!["en-US"]);
        fs::remove_file(file).unwrap();
    }

    #[test]
    fn reports_jsonc_syntax_errors() {
        let file = temp_file("json");
        fs::write(&file, r#"{ "spellcheck": }"#).unwrap();

        let cfg = Config::load(file.clone());
        assert!(cfg.parse_error.is_some());
        assert!(cfg.get("spellcheck").is_none()); // invalid input is ignored, defaults still apply
        fs::remove_file(file).unwrap();
    }

    #[test]
    fn missing_config_uses_defaults() {
        let file = temp_file("json");
        let cfg = Config::load(file);
        assert!(cfg.parse_error.is_none());
        assert!(cfg.get("anything").is_none());
    }

    #[test]
    fn state_file_round_trip() {
        let file = temp_file("state");
        let state = StateFile::load(file.clone());
        assert_eq!(state.get_bool("quit-on-close"), None);

        state.set("quit-on-close", Value::Bool(true));
        let reopened = StateFile::load(file.clone());
        assert_eq!(reopened.get_bool("quit-on-close"), Some(true));
        fs::remove_file(file).unwrap();
    }
}
