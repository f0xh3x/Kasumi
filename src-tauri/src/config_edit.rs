//! Reading and editing config.toml for the settings window. Edits go through
//! `toml_edit`, so the user's comments, ordering and formatting survive; every
//! change is validated before it is written, and the file watcher hot-reloads it.

use crate::config::{self, Config};
use serde::Deserialize;
use serde_json::Value as Json;
use std::fs;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value};

/// One step of a path into the config: a key, or an index into an array.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Seg {
    Index(usize),
    Key(String),
}

/// The config file as JSON (what the user wrote; missing keys use defaults).
#[tauri::command]
pub async fn config_read() -> Result<Json, String> {
    let text = fs::read_to_string(config::path()).map_err(|e| e.to_string())?;
    let value: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// One edit: set `path` to `value`, or remove it when `value` is null.
#[derive(Debug, Deserialize)]
pub struct Op {
    pub path: Vec<Seg>,
    pub value: Option<Json>,
}

/// Sets (or, with `value: null`, removes) the item at `path`.
#[tauri::command]
pub async fn config_write(path: Vec<Seg>, value: Option<Json>) -> Result<(), String> {
    write_ops(vec![Op { path, value }])
}

/// Several edits applied together and validated once (e.g. moving a widget
/// from the bar into a notch touches several places).
#[tauri::command]
pub async fn config_write_many(ops: Vec<Op>) -> Result<(), String> {
    write_ops(ops)
}

fn write_ops(ops: Vec<Op>) -> Result<(), String> {
    let file = config::path();
    let text = fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    for op in ops {
        edit(doc.as_item_mut(), &op.path, op.value)?;
    }
    let mut new_text = doc.to_string();
    // toml_edit writes LF; keep the file's own line endings.
    if text.contains("\r\n") {
        new_text = new_text.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    // Never write a config the app can't load.
    let cfg: Config = toml::from_str(&new_text).map_err(|e| format!("invalid setting: {e}"))?;
    cfg.validate()?;
    fs::write(&file, new_text).map_err(|e| e.to_string())
}

/// Replaces the whole file with the built-in default config.
#[tauri::command]
pub async fn config_reset() -> Result<(), String> {
    fs::write(config::path(), config::DEFAULT_CONFIG).map_err(|e| e.to_string())
}

fn edit(root: &mut Item, path: &[Seg], value: Option<Json>) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    edit_item(root, path, value)
}

/// `path` continues inside `item`.
fn edit_item(item: &mut Item, path: &[Seg], value: Option<Json>) -> Result<(), String> {
    match path {
        [Seg::Index(i)] => match (item, value) {
            // Index == length appends (e.g. a new [[notches]] table).
            (Item::ArrayOfTables(a), Some(Json::Object(map))) if *i == a.len() => {
                let mut t = Table::new();
                for (k, v) in map {
                    t.insert(&k, Item::Value(to_value(&v)?));
                }
                a.push(t);
                Ok(())
            }
            (Item::ArrayOfTables(a), None) if *i < a.len() => {
                a.remove(*i);
                Ok(())
            }
            (Item::Value(Value::Array(a)), None) if *i < a.len() => {
                a.remove(*i);
                Ok(())
            }
            (Item::Value(Value::Array(a)), Some(v)) if *i < a.len() => {
                let mut new = to_value(&v)?;
                *new.decor_mut() = a.get(*i).unwrap().decor().clone();
                a.replace(*i, new);
                Ok(())
            }
            _ => Err("index out of range".into()),
        },
        [Seg::Index(i), rest @ ..] => match item {
            Item::ArrayOfTables(a) => edit_table(a.get_mut(*i).ok_or("index out of range")?, rest, value),
            Item::Value(Value::Array(a)) => match a.get_mut(*i) {
                Some(Value::InlineTable(t)) => edit_table(t, rest, value),
                _ => Err("index out of range or not a table".into()),
            },
            _ => Err("indexing into a non-array".into()),
        },
        _ => edit_table(item.as_table_like_mut().ok_or("parent is not a table")?, path, value),
    }
}

/// `path` starts with a key inside `table`.
fn edit_table(table: &mut dyn TableLike, path: &[Seg], value: Option<Json>) -> Result<(), String> {
    let Some((Seg::Key(k), rest)) = path.split_first() else { return Err("expected a key".into()) };
    if rest.is_empty() {
        match value {
            None => {
                table.remove(k);
            }
            Some(v) => {
                let new = to_item(&v, table.get(k))?;
                table.insert(k, new);
            }
        }
        return Ok(());
    }
    if table.get(k).is_none() {
        // Missing intermediate section, e.g. a [widgets.clock] that isn't in the file yet.
        let mut t = Table::new();
        t.set_implicit(true);
        table.insert(k, Item::Table(t));
    }
    edit_item(table.get_mut(k).unwrap(), rest, value)
}

/// JSON → TOML item, shaped like the item it replaces (keeps `[[tables]]` as
/// arrays of tables and a value's trailing comment).
fn to_item(v: &Json, old: Option<&Item>) -> Result<Item, String> {
    match (v, old) {
        (Json::Array(items), Some(Item::ArrayOfTables(_))) => {
            let mut aot = ArrayOfTables::new();
            for item in items {
                let Json::Object(map) = item else { return Err("expected a list of tables".into()) };
                let mut t = Table::new();
                for (k, v) in map {
                    t.insert(k, Item::Value(to_value(v)?));
                }
                aot.push(t);
            }
            Ok(Item::ArrayOfTables(aot))
        }
        (Json::Object(map), Some(Item::Table(_)) | None) if old.is_some() => {
            let mut t = Table::new();
            for (k, v) in map {
                t.insert(k, Item::Value(to_value(v)?));
            }
            Ok(Item::Table(t))
        }
        (_, old) => {
            let mut value = to_value(v)?;
            if let Some(Item::Value(prev)) = old {
                *value.decor_mut() = prev.decor().clone();
            }
            Ok(Item::Value(value))
        }
    }
}

fn to_value(v: &Json) -> Result<Value, String> {
    Ok(match v {
        Json::Bool(b) => (*b).into(),
        Json::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64().ok_or("bad number")?.into(),
        },
        Json::String(s) => s.as_str().into(),
        Json::Array(items) => {
            let mut a = Array::new();
            for item in items {
                a.push(to_value(item)?);
            }
            Value::Array(a)
        }
        Json::Object(map) => {
            let mut t = InlineTable::new();
            for (k, v) in map {
                t.insert(k, to_value(v)?);
            }
            Value::InlineTable(t)
        }
        Json::Null => return Err("null values are not allowed".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn apply(text: &str, path: Vec<Seg>, value: Option<Json>) -> String {
        let mut doc: DocumentMut = text.parse().unwrap();
        edit(doc.as_item_mut(), &path, value).unwrap();
        doc.to_string()
    }
    fn k(s: &str) -> Seg {
        Seg::Key(s.into())
    }

    #[test]
    fn keeps_comments_when_changing_a_value() {
        let src = "[timing]\ndwell_ms = 10      # delay\ngrace_ms = 300\n";
        let out = apply(src, vec![k("timing"), k("dwell_ms")], Some(json!(40)));
        assert_eq!(out, "[timing]\ndwell_ms = 40      # delay\ngrace_ms = 300\n");
    }

    #[test]
    fn creates_missing_widget_section() {
        let src = "[widgets.clock]\nformat = \"%H:%M\"\n";
        let out = apply(src, vec![k("widgets"), k("media"), k("enabled")], Some(json!(false)));
        let v: toml::Value = toml::from_str(&out).unwrap();
        assert_eq!(v["widgets"]["media"]["enabled"].as_bool(), Some(false));
        assert!(out.contains("format = \"%H:%M\""));
    }

    #[test]
    fn edits_inside_array_of_tables() {
        let src = "[[notches]]\nid = \"clock\"\n\n[[notches]]\nid = \"temps\"\noffset = 150 # px\n";
        let out = apply(src, vec![k("notches"), Seg::Index(1), k("offset")], Some(json!(120)));
        assert!(out.contains("offset = 120 # px"), "{out}");
        assert!(out.contains("id = \"clock\""));
    }

    #[test]
    fn replaces_arrays_and_removes_keys() {
        let src = "[bar]\nleft = [\"menu\", \"tray\"]\nright = [\"media\"]\n";
        let out = apply(src, vec![k("bar"), k("left")], Some(json!(["tray", "menu"])));
        let out = apply(&out, vec![k("bar"), k("right")], None);
        let v: toml::Value = toml::from_str(&out).unwrap();
        assert_eq!(v["bar"]["left"].as_array().unwrap().len(), 2);
        assert_eq!(v["bar"]["left"][0].as_str(), Some("tray"));
        assert!(v["bar"].get("right").is_none());
    }

    #[test]
    fn appends_to_array_of_tables() {
        let src = "[[notches]]
id = \"clock\" # main
";
        let out = apply(src, vec![k("notches"), Seg::Index(1)], Some(json!({"id": "weather", "width": 120})));
        let v: toml::Value = toml::from_str(&out).unwrap();
        assert_eq!(v["notches"][1]["id"].as_str(), Some("weather"));
        assert!(out.contains("id = \"clock\" # main"));
    }

    #[test]
    fn default_config_round_trips_unchanged() {
        // Content is preserved exactly; only line endings are normalized (config_write restores them).
        let doc: DocumentMut = config::DEFAULT_CONFIG.parse().unwrap();
        assert_eq!(doc.to_string().replace("\r\n", "\n"), config::DEFAULT_CONFIG.replace("\r\n", "\n"));
    }
}
