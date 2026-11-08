//! File > Machine > Load / Save configuration: the machine (and the display) as JSON, applied
//! through the commands, so a configuration loaded changes the running machine as the menus do.

use serde_json::{json, Value};

use super::{state, Command, CommandError, CommandTarget, COMMANDS};

/// The radio groups a configuration keeps: its key, the items' menus and their ids' start (one
/// checked command each).
const GROUPS: &[(&str, &[&str], &str)] = &[
    ("platform", &["Machine", "Platform"], "machine-platform-"),
    ("crtc", &["Machine", "CRTC"], "machine-crtc-"),
    ("gateArray", &["Machine", "Gate Array"], "machine-ga-"),
    (
        "language",
        &["Machine", "ROM language"],
        "machine-language-",
    ),
    ("brand", &["Machine", "Brand"], "machine-brand-"),
    (
        "discInterface",
        &["Machine", "Disc interface"],
        "machine-disc-interface-",
    ),
    ("ram", &["Machine", "Memory expansion"], "machine-ram-"),
    ("driveA", &["File", "Drive A", "Kind"], "drive-a-"),
    ("driveB", &["File", "Drive B", "Kind"], "drive-b-"),
    ("rendering", &["Video", "Rendering"], "video-rendering-"),
    ("keyboard", &["Machine", "Keyboard"], "keyboard-mapping-"),
    (
        "hostLayout",
        &["Machine", "Keyboard"],
        "keyboard-host-layout-",
    ),
    ("speed", &["Debug", "Speed"], "speed-"),
    ("audio", &["Audio"], "audio-"),
];

/// The defaults: a 6128, CRTC 0, a GA 40010, the raw display (File > Machine > Restore).
const DEFAULTS: &[(&str, &str)] = &[
    ("platform", "machine-platform-cpc6128"),
    ("crtc", "machine-crtc-0"),
    ("gateArray", "machine-ga-40010"),
    ("language", "machine-language-en"),
    ("brand", "machine-brand-amstrad"),
    ("discInterface", "machine-disc-interface-default"),
    ("ram", "machine-ram-default"),
    ("driveA", "drive-a-original"),
    ("driveB", "drive-b-original"),
    ("rendering", "video-rendering-raw"),
    ("keyboard", "keyboard-mapping-symbolic"),
    ("hostLayout", "keyboard-host-layout-auto"),
    ("speed", "speed-normal"),
    ("audio", "audio-stereo"),
];

/// The radio items of a group.
fn radio_items<'a>(
    menus: &'a [&'a str],
    prefix: &'a str,
) -> impl Iterator<Item = &'static str> + 'a {
    COMMANDS
        .iter()
        .filter(move |spec| {
            spec.kind == super::Kind::Radio && spec.menus() == menus && spec.id.starts_with(prefix)
        })
        .map(|spec| spec.id)
}

/// The configuration of `target`, as JSON.
pub fn save(target: &dyn CommandTarget) -> String {
    let mut config = serde_json::Map::new();
    for (key, menus, prefix) in GROUPS {
        if let Some(id) = radio_items(menus, prefix).find(|id| state(id, target).checked) {
            config.insert((*key).to_owned(), json!(id));
        }
    }
    if let Some(settings) = target.settings() {
        config.insert("curvature".into(), json!(settings.curvature));
        config.insert("blackLevel".into(), json!(settings.black_level));
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
    // the platform first: it sets the CRTC of a CPC+ and the language back
    for (key, default) in DEFAULTS {
        let (_, menus, prefix) = GROUPS
            .iter()
            .find(|(k, _, _)| k == key)
            .expect("a default per group");
        let id = match config.get(*key).and_then(Value::as_str) {
            Some(id) if radio_items(menus, prefix).any(|item| item == id) => id,
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
    }
    Ok(notes)
}
