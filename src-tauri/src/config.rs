use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use jsonc_parser::ast::{Object, Value as AstValue};
use jsonc_parser::common::Ranged;
use jsonc_parser::{parse_to_ast, parse_to_serde_value, CollectOptions, ParseOptions};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

/// A JSONC configuration file, compatible with the one written by the Electron
/// version. Comments and formatting are preserved when editing an existing key.
pub struct JsonConfig {
    path: PathBuf,
    raw: String,
    data: Value,
    parse_error: Option<String>,
}

impl JsonConfig {
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut config = Self {
            path,
            raw: "{}".to_string(),
            data: Value::Object(Map::new()),
            parse_error: None,
        };

        if !config.path.exists() {
            return config;
        }

        match fs::read_to_string(&config.path) {
            Ok(raw) => {
                config.raw = raw;
                config.parse();
            }
            Err(err) => {
                config.parse_error = Some(format!("cannot read file: {err}"));
            }
        }

        config
    }

    pub fn parse_error(&self) -> Option<&str> {
        self.parse_error.as_deref()
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str, default: T) -> T {
        self.data
            .get(key)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or(default)
    }

    pub fn get_opt_string(&self, key: &str) -> Option<String> {
        self.data.get(key)?.as_str().map(str::to_owned)
    }

    pub fn get_string(&self, key: &str, default: &str) -> String {
        self.get_opt_string(key)
            .unwrap_or_else(|| default.to_string())
    }

    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.data
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(default)
    }

    pub fn set<V: Serialize>(&mut self, key: &str, value: V) -> Result<()> {
        let value = serde_json::to_value(value).context("serialize config value")?;

        let candidate = match edit_text(&self.raw, key, &value) {
            Ok(candidate) => candidate,
            Err(err) => {
                tracing::warn!(
                    "cannot preserve formatting while writing {key}: {err:#}; rewriting the file"
                );
                self.render_with(key, &value)?
            }
        };

        let parsed = parse_to_serde_value(&candidate, &ParseOptions::default())
            .map_err(|err| anyhow!("edited config is not valid JSONC: {err}"))?;
        let data = match parsed {
            Some(Value::Object(map)) => Value::Object(map),
            Some(_) => return Err(anyhow!("edited config is not a JSON object")),
            None => Value::Object(Map::new()),
        };

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create config directory {}", parent.display()))?;
        }
        fs::write(&self.path, &candidate)
            .with_context(|| format!("write config file {}", self.path.display()))?;

        self.raw = candidate;
        self.data = data;
        self.parse_error = None;
        Ok(())
    }

    fn parse(&mut self) {
        if self.raw.trim().is_empty() {
            self.data = Value::Object(Map::new());
            self.parse_error = None;
            return;
        }

        match parse_to_serde_value(&self.raw, &ParseOptions::default()) {
            Ok(Some(Value::Object(map))) => {
                self.data = Value::Object(map);
                self.parse_error = None;
            }
            Ok(Some(_)) => {
                self.data = Value::Object(Map::new());
                self.parse_error = Some("top-level value must be an object".to_string());
            }
            Ok(None) => {
                self.data = Value::Object(Map::new());
                self.parse_error = None;
            }
            Err(err) => {
                self.data = Value::Object(Map::new());
                self.parse_error = Some(err.to_string());
            }
        }
    }

    fn render_with(&self, key: &str, value: &Value) -> Result<String> {
        let mut data = match &self.data {
            Value::Object(map) => map.clone(),
            _ => Map::new(),
        };
        data.insert(key.to_string(), value.clone());

        let mut rendered =
            serde_json::to_string_pretty(&Value::Object(data)).context("render config")?;
        rendered.push('\n');
        Ok(rendered)
    }
}

/// Replace the value of `key` in place, or insert it before the closing brace,
/// keeping comments and the rest of the file untouched.
fn edit_text(raw: &str, key: &str, value: &Value) -> Result<String> {
    let text = if raw.trim().is_empty() { "{}" } else { raw };
    let parsed = parse_to_ast(text, &CollectOptions::default(), &ParseOptions::default())
        .map_err(|err| anyhow!("{err}"))?;

    let root: Object<'_> = match parsed.value {
        Some(AstValue::Object(object)) => object,
        Some(_) => return Err(anyhow!("top-level value is not an object")),
        None => return Err(anyhow!("empty document")),
    };

    if let Some(prop) = root.get(key) {
        let indent = line_indent(text, prop.range.start);
        let serialized = serialize_value(value, indent);
        let range = prop.value.range();

        let mut output = String::with_capacity(text.len() + serialized.len());
        output.push_str(&text[..range.start]);
        output.push_str(&serialized);
        output.push_str(&text[range.end..]);
        return Ok(output);
    }

    let parent_indent = line_indent(text, root.range.start);
    let prop_indent = format!("{parent_indent}  ");
    let serialized = serialize_value(value, &prop_indent);
    let key = serde_json::to_string(key).context("serialize config key")?;

    let mut output = String::with_capacity(text.len() + serialized.len() + key.len() + 8);
    if let Some(last) = root.properties.last() {
        // Insert right after the last value; the comma that used to precede the
        // closing brace (if any) ends up after the new property, which is valid
        // JSONC either way.
        let pos = last.range().end;
        output.push_str(&text[..pos]);
        output.push_str(&format!(",\n{prop_indent}{key}: {serialized}"));
        output.push_str(&text[pos..]);
    } else {
        let pos = root.range.start + 1;
        output.push_str(&text[..pos]);
        output.push_str(&format!("\n{prop_indent}{key}: {serialized}"));
        output.push_str(&text[pos..]);
    }

    Ok(output)
}

fn serialize_value(value: &Value, indent: &str) -> String {
    let pretty = serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string());
    if indent.is_empty() {
        return pretty;
    }

    let mut output = String::with_capacity(pretty.len());
    for (index, line) in pretty.split('\n').enumerate() {
        if index > 0 {
            output.push('\n');
            output.push_str(indent);
        }
        output.push_str(line);
    }
    output
}

fn line_indent(text: &str, position: usize) -> &str {
    let line_start = text[..position]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let prefix = &text[line_start..position];
    let trimmed = prefix.trim_start_matches([' ', '\t']);
    &prefix[..prefix.len() - trimmed.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gets_defaults_and_values() {
        let config = JsonConfig {
            path: PathBuf::from("/tmp/config.json"),
            raw: r#"{
  // comment
  "quit-on-close": true,
  "log-level": "debug",
}"#
            .to_string(),
            data: Value::Object(Map::new()),
            parse_error: None,
        };
        let mut config = config;
        config.parse();

        assert_eq!(config.parse_error(), None);
        assert!(config.get_bool("quit-on-close", false));
        assert_eq!(config.get_string("log-level", "info"), "debug");
        assert_eq!(config.get_string("missing", "info"), "info");
    }

    #[test]
    fn replaces_value_and_keeps_comments() {
        let raw = r#"{
  // keep me
  "quit-on-close": false,
  "log-level": "info"
}"#;
        let edited = edit_text(raw, "quit-on-close", &Value::Bool(true)).unwrap();
        assert!(edited.contains("// keep me"));
        assert!(edited.contains("\"quit-on-close\": true"));
        assert!(parse_to_serde_value(&edited, &ParseOptions::default()).is_ok());
    }

    #[test]
    fn inserts_into_empty_object() {
        let edited = edit_text("{}", "window-bounds", &serde_json::json!({"width": 1})).unwrap();
        assert!(edited.contains("\"window-bounds\""));
        assert!(parse_to_serde_value(&edited, &ParseOptions::default()).is_ok());
    }

    #[test]
    fn inserts_with_trailing_comma() {
        let raw = "{\n  \"a\": 1,\n}\n";
        let edited = edit_text(raw, "b", &Value::Bool(true)).unwrap();
        assert!(parse_to_serde_value(&edited, &ParseOptions::default()).is_ok());
        let parsed = parse_to_serde_value(&edited, &ParseOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(parsed["b"], Value::Bool(true));
    }

    #[test]
    fn reports_parse_errors() {
        let mut config = JsonConfig {
            path: PathBuf::from("/tmp/config.json"),
            raw: "{ invalid".to_string(),
            data: Value::Object(Map::new()),
            parse_error: None,
        };
        config.parse();

        assert!(config.parse_error().is_some());
        assert!(config.data.as_object().is_some_and(Map::is_empty));
    }

    #[test]
    fn replaces_nested_object_and_keeps_comments() {
        let raw = r#"{
  // window state
  "window-bounds": {
    "x": 22,
    "y": 19,
    "width": 1717,
    "height": 937
  }
}"#;
        let value = serde_json::json!({
            "x": 1.0,
            "y": 2.0,
            "width": 1000.0,
            "height": 800.0
        });
        let edited = edit_text(raw, "window-bounds", &value).unwrap();

        assert!(edited.contains("// window state"));
        assert!(!edited.contains("1717"));
        let parsed = parse_to_serde_value(&edited, &ParseOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(parsed["window-bounds"]["width"], 1000.0);
    }
}
