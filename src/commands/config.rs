//! File > Machine > Load / Save configuration: the machine (and the display) as JSON, applied
//! through the commands, so a configuration loaded changes the running machine as the menus do.

use serde_json::{json, Value};

use super::{
    state, Arg, ArgKind, Command, CommandError, CommandTarget, ConfigGroup, Outcome, CONFIG_GROUPS,
    CONFIG_VALUES,
};

/// The configuration of `target`, as JSON.
pub fn save(target: &mut dyn CommandTarget) -> String {
    let mut config = serde_json::Map::new();
    for group in CONFIG_GROUPS {
        if let Some(id) = group.items.iter().find(|id| state(id, target).checked) {
            config.insert(group.key.to_owned(), json!(id));
        }
    }
    for value in CONFIG_VALUES {
        if let Some(current) = current_value(target, value.id) {
            config.insert(value.key.to_owned(), current);
        }
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
    for value in CONFIG_VALUES {
        let Some(number) = config.get(value.key).and_then(Value::as_f64) else {
            continue;
        };
        // the input's own range applies: a value out of it is clamped there
        let Some(arg) = (match current_kind(target, value.id) {
            Some(ArgKind::Number { .. }) => Some(Arg::Number(number.round() as i64)),
            Some(ArgKind::Decimal { .. }) => Some(Arg::Text(number.to_string())),
            _ => None,
        }) else {
            continue;
        };
        match Command::with_arg(value.id, arg)?.execute(target) {
            Ok(_) | Err(CommandError::Unsupported) => {}
            Err(error) => notes.push(format!("{}: {error}", value.key)),
        }
    }
    Ok(notes)
}

/// What the input `id` asks for: run without its argument, it only describes the question, its
/// default being the value now (nothing is changed).
fn current_kind(target: &mut dyn CommandTarget, id: &str) -> Option<ArgKind> {
    match Command::new(id).ok()?.execute(target) {
        Ok(Outcome::NeedsArg(prompt)) => Some(prompt.kind),
        _ => None,
    }
}

/// The value the input `id` has now, as JSON.
fn current_value(target: &mut dyn CommandTarget, id: &str) -> Option<Value> {
    match current_kind(target, id)? {
        ArgKind::Number { default, .. } => Some(json!(default)),
        ArgKind::Decimal { default, .. } => Some(json!(default)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::HostSettings;
    use crate::motherboard::Motherboard;

    struct Target(Motherboard, HostSettings);

    impl CommandTarget for Target {
        fn motherboard(&self) -> &Motherboard {
            &self.0
        }
        fn motherboard_mut(&mut self) -> &mut Motherboard {
            &mut self.0
        }
        fn settings(&self) -> Option<&HostSettings> {
            Some(&self.1)
        }
        fn settings_mut(&mut self) -> Option<&mut HostSettings> {
            Some(&mut self.1)
        }
        fn has_file_system(&self) -> bool {
            false
        }
    }

    fn target() -> Target {
        Target(Motherboard::default(), HostSettings::default())
    }

    #[test]
    fn the_values_go_through_their_commands_and_back() {
        let mut target = target();
        let json = load(&mut target, r#"{"brightness": 40, "hHold": -30, "curvature": 2.5}"#);
        assert_eq!(json.unwrap(), Vec::<String>::new());
        assert_eq!(target.1.brightness, 40);
        assert_eq!(target.1.hhold, -30);
        assert_eq!(target.1.curvature, 2.5);
        let saved: Value = serde_json::from_str(&save(&mut target)).unwrap();
        assert_eq!(saved["brightness"], 40);
        assert_eq!(saved["hHold"], -30);
        assert_eq!(saved["curvature"], 2.5);
    }

    #[test]
    fn a_value_out_of_range_is_clamped_by_its_command() {
        let mut target = target();
        load(&mut target, r#"{"brightness": 900, "vHold": -900, "curvature": 99}"#).unwrap();
        assert_eq!(target.1.brightness, 100);
        assert_eq!(target.1.vhold, -100);
        assert_eq!(target.1.curvature, 4.0);
    }

    #[test]
    fn every_group_and_value_of_the_specification_is_saved() {
        let mut target = target();
        let saved: Value = serde_json::from_str(&save(&mut target)).unwrap();
        for value in CONFIG_VALUES {
            assert!(saved.get(value.key).is_some(), "{} is not saved", value.key);
        }
        for group in CONFIG_GROUPS {
            assert!(group.items.contains(&group.default), "{}", group.key);
        }
    }
}
