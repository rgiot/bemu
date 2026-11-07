//! File > Machine > Load / Save configuration: the machine (and the display) as JSON, applied
//! through the commands, so a configuration loaded changes the running machine as the menus do.

use serde_json::{json, Value};

use super::{state, Command, CommandError, CommandTarget, COMMANDS};

/// The radio groups a configuration keeps, by their menus: one checked command each.
const GROUPS: &[(&str, &[&str])] = &[
    ("platform", &["Machine", "Platform"]),
    ("crtc", &["Machine", "CRTC"]),
    ("gateArray", &["Machine", "Gate Array"]),
    ("language", &["Machine", "ROM language"]),
    ("brand", &["Machine", "Brand"]),
    ("discInterface", &["Machine", "Disc interface"]),
    ("ram", &["Machine", "Memory expansion"]),
    ("driveA", &["File", "Drive A", "Kind"]),
    ("driveB", &["File", "Drive B", "Kind"]),
    ("rendering", &["Video", "Rendering"]),
    ("keyboard", &["Machine", "Keyboard"]),
    ("speed", &["Debug", "Speed"]),
    ("audio", &["Audio"]),
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
    ("speed", "speed-normal"),
    ("audio", "audio-stereo"),
];

fn radio_items<'a>(menus: &'a [&'a str]) -> impl Iterator<Item = &'static str> + 'a {
    COMMANDS
        .iter()
        .filter(move |spec| spec.kind == super::Kind::Radio && spec.menus() == menus)
        .map(|spec| spec.id)
}

/// The configuration of `target`, as JSON.
pub fn save(target: &dyn CommandTarget) -> String {
    let mut config = serde_json::Map::new();
    for (key, menus) in GROUPS {
        if let Some(id) = radio_items(menus).find(|id| state(id, target).checked) {
            config.insert((*key).to_owned(), json!(id));
        }
    }
    if let Some(settings) = target.settings() {
        config.insert("curvature".into(), json!(settings.curvature));
        config.insert("blackLevel".into(), json!(settings.black_level));
    }
    serde_json::to_string_pretty(&Value::Object(config)).unwrap()
}

/// Apply the configuration `text` (what it leaves out goes back to the defaults).
pub fn load(target: &mut dyn CommandTarget, text: &str) -> Result<(), CommandError> {
    let config: Value = serde_json::from_str(text)
        .map_err(|e| CommandError::Invalid(format!("configuration: {e}")))?;
    // the platform first: it sets the CRTC of a CPC+ and the language back
    for (key, default) in DEFAULTS {
        let id = config.get(*key).and_then(Value::as_str).unwrap_or(default);
        let command = Command::new(id)?;
        if !state(id, target).enabled || state(id, target).checked {
            continue;
        }
        match command.execute(target) {
            Ok(_) | Err(CommandError::Unsupported) => {}
            Err(error) => return Err(error),
        }
    }
    if let Some(settings) = target.settings_mut() {
        if let Some(curvature) = config.get("curvature").and_then(Value::as_f64) {
            settings.curvature = curvature as f32;
        }
        if let Some(level) = config.get("blackLevel").and_then(Value::as_u64) {
            settings.black_level = level.min(255) as u8;
        }
    }
    Ok(())
}
