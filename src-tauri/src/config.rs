//! ~/.config/elecwhat/config.json (JSONC, read-only) and persistent-state.json (read/write).

use jsonc_parser::{parse_to_serde_value, ParseOptions};
use serde_json::{Map, Value};
use std::{fs, path::PathBuf, sync::Mutex};

pub struct Config {
    pub file: PathBuf,
    data: Map<String, Value>,
    pub parse_error: Option<String>,
}

impl Config {
    pub fn load(file: PathBuf) -> Self {
        let mut cfg = Config { file, data: Map::new(), parse_error: None };
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
        self.get(key).and_then(Value::as_str).unwrap_or(default).to_string()
    }

    pub fn u64(&self, key: &str, default: u64) -> u64 {
        self.get(key).and_then(Value::as_u64).unwrap_or(default)
    }

    pub fn str_list(&self, key: &str) -> Option<Vec<String>> {
        self.get(key).and_then(Value::as_array).map(|a| {
            a.iter().filter_map(|v| v.as_str().map(String::from)).collect()
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
        StateFile { file, data: Mutex::new(data) }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.data.lock().unwrap().get(key).and_then(Value::as_bool)
    }

    pub fn set(&self, key: &str, value: Value) {
        let mut data = self.data.lock().unwrap();
        data.insert(key.to_string(), value);
        if let Ok(text) = serde_json::to_string_pretty(&*data) {
            if let Err(e) = fs::write(&self.file, text) {
                log::error!("cannot save {}: {e}", self.file.display());
            }
        }
    }
}
