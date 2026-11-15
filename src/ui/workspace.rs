//! The workspace: the windows open, where they are, and each one's own settings (a Memory
//! window's address, a Disassembly window's bookmarks...), kept between two sessions in
//! `workspace.json` of the user's configuration folder.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where a window of the system is: its position (physical pixels) and its size (logical
/// pixels), only when the user sized it (else it is fitted to its content).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub position: [i32; 2],
    #[serde(default)]
    pub size: Option<[f32; 2]>,
}

/// One window of the workspace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    /// The command that opens its view (`view-memory`): stable from a version to the next.
    pub view: String,
    /// Which of its view's windows (Memory 2: 1).
    #[serde(default)]
    pub copy: u8,
    /// Open when the session ended (else its settings only, for when it opens again).
    #[serde(default)]
    pub open: bool,
    #[serde(default)]
    pub geometry: Option<Geometry>,
    #[serde(default)]
    pub settings: serde_json::Value,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub windows: Vec<SavedWindow>,
}

/// The folder of bemu's settings: `$BEMU_CONFIG_DIR`, else the system's (`$XDG_CONFIG_HOME/bemu`
/// or `~/.config/bemu`, `~/Library/Application Support/bemu`, `%APPDATA%\bemu`).
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("BEMU_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("bemu"))
    } else if cfg!(target_os = "macos") {
        home().map(|home| home.join("Library/Application Support/bemu"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|home| home.join(".config")))
            .map(|dir| dir.join("bemu"))
    }
}

fn path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("workspace.json"))
}

/// The workspace of the last session (`None`: none, or unreadable).
pub fn load() -> Option<Workspace> {
    let text = std::fs::read_to_string(path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Keep `workspace` for the next session.
pub fn save(workspace: &Workspace) -> Result<(), String> {
    let path = path().ok_or("no configuration folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(workspace).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}
