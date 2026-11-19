//! File > Machine > Load / Save configuration: the machine (and the display) as JSON, applied
//! through the commands, so a configuration loaded changes the running machine as the menus do.

use serde_json::{json, Value};

use super::{state, Command, CommandError, CommandTarget, ConfigGroup, CONFIG_GROUPS};

/// The configuration of `target`, as JSON.
pub fn save(target: &dyn CommandTarget) -> String {
    let mut config = serde_json::Map::new();
    for group in CONFIG_GROUPS {
        if let Some(id) = group.items.iter().find(|id| state(id, target).checked) {
            config.insert(group.key.to_owned(), json!(id));
        }
    }
    if let Some(settings) = target.settings() {
        config.insert("curvature".into(), json!(settings.curvature));
        config.insert("blackLevel".into(), json!(settings.black_level));
        config.insert("brightness".into(), json!(settings.brightness));
        config.insert("hHold".into(), json!(settings.hhold));
        config.insert("vHold".into(), json!(settings.vhold));
    }
    serde_json::to_string_pretty(&Value::Object(config)).unwrap()
}

/// Apply the configuration `text` (what it leaves out goes back to the defaults). Only the
/// items of each group are run (a configuration cannot run any other command); a value that is
/// not one of them is reported and left out.
pub fn load(target: &mut dyn CommandTarget, text: &str) -> Result<Vec<String>, CommandError> {
    let config: Value = serde_json::from_str(text)
        .map_err(|e| CommandError::Invalid(format!("configuration: {e}")))?;
    let mut notes = Vec::new();
    // in the order of the specification's `Config` column: the platform first, it sets the CRTC
    // of a CPC+ and the language back
    for ConfigGroup { key, items, default } in CONFIG_GROUPS {
        let id = match config.get(*key).and_then(Value::as_str) {
            Some(id) if items.contains(&id) => id,
            Some(other) => {
                notes.push(format!("{key}: {other} is not one of its values"));
                default
            }
            None => default,
        };
        let state = state(id, target);
        if !state.enabled || state.checked {
            continue;
        }
        match Command::new(id)?.execute(target) {
            Ok(_) | Err(CommandError::Unsupported) => {}
            Err(error) => notes.push(format!("{key}: {error}")),
        }
    }
    if let Some(settings) = target.settings_mut() {
        if let Some(curvature) = config.get("curvature").and_then(Value::as_f64) {
            settings.curvature = (curvature as f32).clamp(0.0, 4.0);
        }
        if let Some(level) = config.get("blackLevel").and_then(Value::as_u64) {
            settings.black_level = level.min(255) as u8;
        }
        if let Some(level) = config.get("brightness").and_then(Value::as_i64) {
            settings.brightness = level.clamp(-100, 100) as i8;
        }
        if let Some(hold) = config.get("hHold").and_then(Value::as_i64) {
            settings.hhold = hold.clamp(-100, 100) as i8;
        }
        if let Some(hold) = config.get("vHold").and_then(Value::as_i64) {
            settings.vhold = hold.clamp(-100, 100) as i8;
        }
    }
    Ok(notes)
}
