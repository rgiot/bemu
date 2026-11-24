//! Help > Third-party licences: what this build contains and is not bemu's own (the ROMs, the
//! libraries), each with its licence's text, from `third_party/licenses.json` (written by
//! `tools/licenses.py`, see NOTICE.md).
//!
//! The inventory is embedded gzipped (build.rs), and decompressed when the program starts
//! ([`preload`], on a thread of its own where there are threads).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use super::window::{Cx, ViewWindow};

/// The inventory, embedded gzipped (build.rs).
const INVENTORY_GZ: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/licenses.json.gz"));

/// This build, as the inventory names it.
#[cfg(not(target_arch = "wasm32"))]
const BUILD: &str = "desktop";
#[cfg(target_arch = "wasm32")]
const BUILD: &str = "web";

/// This build's licence, in a line (the About window's).
#[cfg(not(target_arch = "wasm32"))]
pub const BUILD_LICENCE: &str =
    "Source: MIT or Apache-2.0. This program: GNU GPL 3.0 (it links the HxC floppy library).";
#[cfg(target_arch = "wasm32")]
pub const BUILD_LICENCE: &str = "Licence: MIT or Apache-2.0.";

/// The GPL's text, the licence of the desktop executables.
#[cfg(not(target_arch = "wasm32"))]
const GPL: Option<&str> = Some(include_str!("../../LICENSE-GPL-3.0"));
#[cfg(target_arch = "wasm32")]
const GPL: Option<&str> = None;

const SOURCE: &str = "https://github.com/rgiot/bemu";

#[derive(Deserialize)]
pub struct Inventory {
    pub components: Vec<Component>,
    /// The licence texts, by the keys the components give.
    pub texts: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub struct Component {
    /// "rom", "c-library", "crate" or "android".
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub license: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    pub builds: Vec<String>,
    pub texts: Vec<Text>,
}

#[derive(Deserialize)]
pub struct Text {
    pub name: String,
    /// The key of the text in [`Inventory::texts`].
    pub text: String,
}

static INVENTORY: OnceLock<Inventory> = OnceLock::new();

/// The inventory, decompressed and read (once).
pub fn inventory() -> &'static Inventory {
    INVENTORY.get_or_init(|| {
        let mut json = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(INVENTORY_GZ), &mut json)
            .expect("the embedded licences");
        serde_json::from_slice(&json).expect("third_party/licenses.json")
    })
}

/// The inventory decompressed now, the program starting: on a thread of its own on the desktop
/// (the window opens meanwhile); in a page, where there are no threads, when it is first shown.
pub fn preload() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let _ = std::thread::Builder::new().name("licences".to_owned()).spawn(|| {
                inventory();
            });
        });
    }
}

/// The components of this build.
pub fn of_this_build() -> impl Iterator<Item = &'static Component> {
    inventory()
        .components
        .iter()
        .filter(|component| component.builds.iter().any(|build| build == BUILD))
}

fn kind_title(kind: &str) -> &'static str {
    match kind {
        "rom" => "ROMs",
        "c-library" => "C and Z80 code",
        "crate" => "Rust libraries",
        "android" => "Android libraries",
        _ => "Other",
    }
}

/// The window: a filter, and the components.
#[derive(Default)]
pub struct Licences {
    filter: String,
}

impl ViewWindow for Licences {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut Cx) {
        ui.label(BUILD_LICENCE);
        ui.horizontal(|ui| {
            ui.label("Source:");
            ui.hyperlink(SOURCE);
        });
        ui.label("What follows is in this program and is not bemu's own: each keeps its owners' licence.");
        if GPL.is_some() {
            ui.label("Because of the ROMs' terms and part of HxC, this program must be given away, not sold.");
        }
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(&mut self.filter);
        });
        ui.separator();
        let filter = self.filter.to_lowercase();
        let inventory = inventory();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(gpl) = GPL {
                egui::CollapsingHeader::new("This program: GNU General Public License, version 3")
                    .id_salt("licence-gpl")
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(gpl).monospace().small());
                    });
            }
            let mut kind = "";
            for component in of_this_build() {
                let shown = filter.is_empty()
                    || component.name.to_lowercase().contains(&filter)
                    || component.license.to_lowercase().contains(&filter)
                    || component.owner.as_deref().unwrap_or("").to_lowercase().contains(&filter);
                if !shown {
                    continue;
                }
                if component.kind != kind {
                    kind = &component.kind;
                    ui.add_space(6.0);
                    ui.strong(kind_title(kind));
                }
                let title = if component.version.is_empty() {
                    format!("{} — {}", component.name, component.license)
                } else {
                    format!("{} {} — {}", component.name, component.version, component.license)
                };
                egui::CollapsingHeader::new(title)
                    .id_salt(("licence", &component.kind, &component.name, &component.version))
                    .show(ui, |ui| {
                        if let Some(owner) = &component.owner {
                            ui.label(format!("By {owner}"));
                        }
                        if let Some(url) = &component.url {
                            ui.hyperlink(url);
                        }
                        if let Some(note) = &component.note {
                            ui.label(note);
                        }
                        for text in &component.texts {
                            let body = inventory.texts.get(&text.text).map_or("", String::as_str);
                            egui::CollapsingHeader::new(&text.name)
                                .id_salt(("licence-text", &component.name, &component.version, &text.name))
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new(body).monospace().small());
                                });
                        }
                    });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inventory_lists_this_build_with_its_texts() {
        let inventory = inventory();
        let listed: Vec<&Component> = of_this_build().collect();
        assert!(listed.len() > 100, "{} components", listed.len());
        assert!(listed.iter().any(|c| c.kind == "rom" && c.name.contains("Amstrad")));
        if BUILD == "desktop" {
            assert!(listed.iter().any(|c| c.name == "hxcfe-sys" && c.license == "GPL-3.0-only"));
        }
        // bemu's and cpclib's own are not third-party
        assert!(!listed.iter().any(|c| c.name.starts_with("cpclib") || c.name == "amsrust"));
        for component in &inventory.components {
            assert!(!component.texts.is_empty(), "{} has no licence text", component.name);
            for text in &component.texts {
                assert!(inventory.texts.contains_key(&text.text), "{}: {}", component.name, text.name);
            }
        }
    }
}
