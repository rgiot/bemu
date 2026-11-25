//! The app's settings (JSON from Kotlin, camelCase), and the machine they make.
//!
//! The defaults are the app's: a 6128 seen on a CTM644 (its CRT drawn by the GPU), the M4 board
//! with its WiFi and RulezCharge (|RZ: games and demos from cpcrulez.fr, downloaded on the SD
//! card), UniDOS on top of AMSDOS, and the media started by themselves (autorun).

use amsrust::display::DisplayMode;
use amsrust::motherboard::{CpcModel, Motherboard, MotherboardBuilder};
use amsrust::roms::load_rom;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Config {
    /// "464", "664", "6128", "6128plus", "464plus" or "gx4000".
    pub model: String,
    /// The CRTC type, 0-4 (none: the model's).
    pub crtc: Option<u8>,
    /// "en" or "fr" (the 6128's French ROMs and AZERTY keyboard).
    pub language: String,
    /// "ctm" (the CTM644 colour monitor), "gt65" (green), "mm12" (grey), "trinitron", "atari" or
    /// "raw" (the picture without a monitor: the cheapest).
    pub display: String,
    /// How curved the glass is, 0 (flat) to 4.
    pub curvature: f32,
    /// The M4 board, its SD card the app's folder.
    pub m4: bool,
    /// The M4's WiFi (the phone's network).
    pub m4_wifi: bool,
    /// RulezCharge in slot 5 (|RZ), with the M4's WiFi.
    pub rulezcharge: bool,
    /// UniDOS (and its M4 node) above AMSDOS.
    pub unidos: bool,
    /// The PlayCity (two more AYs).
    pub playcity: bool,
    /// A disc or a tape is started by itself once the machine has booted.
    pub autorun: bool,
    /// Sound.
    pub audio: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: "6128".to_owned(),
            crtc: None,
            language: "en".to_owned(),
            display: "ctm".to_owned(),
            curvature: 1.0,
            m4: true,
            m4_wifi: true,
            rulezcharge: true,
            unidos: true,
            playcity: false,
            autorun: true,
            audio: true,
        }
    }
}

impl Config {
    /// From the app's JSON; an empty string is the defaults.
    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(json).map_err(|e| format!("settings: {e}"))
    }

    pub fn display(&self) -> Result<DisplayMode, String> {
        self.display.parse()
    }

    pub fn curvature(&self) -> f32 {
        self.curvature.clamp(0.0, 4.0)
    }

    pub fn model(&self) -> Result<CpcModel, String> {
        self.model.parse::<CpcModel>()
    }

    /// Settings that determine the motherboard, its ROMs and expansions.
    pub fn requires_restart(&self, other: &Self) -> bool {
        self.model != other.model
            || self.crtc != other.crtc
            || self.language != other.language
            || self.m4 != other.m4
            || self.m4_wifi != other.m4_wifi
            || self.rulezcharge != other.rulezcharge
            || self.unidos != other.unidos
            || self.playcity != other.playcity
    }

    /// The machine, `model` replacing the configured one (a cartridge needs a CPC+), with its
    /// expansions and ROMs; `sd`: the M4's SD card folder.
    pub fn build(&self, model: Option<CpcModel>, sd: &std::path::Path) -> Result<Motherboard, String> {
        let model = match model {
            Some(model) => model,
            None => self.model()?,
        };
        let mut builder = MotherboardBuilder::default();
        builder.enable_interruptions();
        builder.set_model(model);
        if let Some(crtc) = self.crtc {
            builder.set_crtc(
                amsrust::crtc::CrtcType::try_from(crtc).map_err(|_| format!("crtc must be 0-4, not {crtc}"))?,
            );
        }
        match self.language.as_str() {
            "en" | "" => {}
            "fr" if model == CpcModel::Cpc6128 => {
                builder.set_keyboard_layout(amsrust::keyboard::KeyboardLayout::French);
            }
            "fr" => {}
            other => return Err(format!("language must be en or fr, not {other:?}")),
        }
        if self.playcity {
            builder.plug_playcity();
        }
        let mut motherboard = builder.build();
        // the drives of the 6128 and the CPC+
        let disc = motherboard.has_disc_interface();
        if self.m4 {
            std::fs::create_dir_all(sd).map_err(|e| format!("{}: {e}", sd.display()))?;
            let slot = if self.unidos && disc {
                amsrust::m4::M4_ROM_SLOT_WITH_UNIDOS
            } else {
                amsrust::m4::M4_ROM_SLOT
            };
            let board = amsrust::m4::M4Board::with_bundled_rom(sd)?
                .in_slot(slot)
                .with_wifi(self.m4_wifi);
            motherboard.plug_expansion(Box::new(board));
        }
        for (slot, rom) in self.roms(disc) {
            motherboard.set_upper_rom(slot, Some(&load_rom(rom)?))?;
        }
        motherboard.restart();
        Ok(motherboard)
    }

    /// The upper ROMs (slot, inner name) besides the model's.
    fn roms(&self, disc: bool) -> Vec<(u8, &'static str)> {
        let mut roms = Vec::new();
        // UniDOS replaces AMSDOS's commands, a disc interface needed
        if self.unidos && disc {
            roms.extend([(7, "inner://unidos"), (14, "inner://unitools"), (15, "inner://amsdos")]);
            if self.m4 {
                roms.push((1, "inner://unidos-m4"));
            }
        }
        if self.m4 && self.m4_wifi && self.rulezcharge {
            roms.push((5, "inner://rulezcharge"));
        }
        roms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sd() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("bemu-droid-sd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_defaults_make_the_droid_machine() {
        let config = Config::from_json("").unwrap();
        assert_eq!(config, Config::default());
        assert!(!config.display().unwrap().is_raw(), "the CTM by default");
        let mb = config.build(None, &sd()).unwrap();
        assert!(mb.has_expansion::<amsrust::m4::M4Board>());
        assert!(mb.upper_rom(5).is_some(), "RulezCharge");
        assert!(mb.upper_rom(1).is_some(), "UniDOS's M4 node");
        assert_eq!(amsrust::autorun::boot_seconds(&mb), amsrust::autorun::AUTORUN_EXPANDED_AT_SECONDS);
    }

    #[test]
    fn only_motherboard_settings_require_restart() {
        let config = Config::default();
        let live = [
            Config { display: "raw".into(), ..config.clone() },
            Config { curvature: 0.0, ..config.clone() },
            Config { audio: false, ..config.clone() },
            Config { autorun: false, ..config.clone() },
        ];
        let hardware = [
            Config { model: "464".into(), ..config.clone() },
            Config { crtc: Some(1), ..config.clone() },
            Config { language: "fr".into(), ..config.clone() },
            Config { m4: false, ..config.clone() },
            Config { m4_wifi: false, ..config.clone() },
            Config { rulezcharge: false, ..config.clone() },
            Config { unidos: false, ..config.clone() },
            Config { playcity: true, ..config.clone() },
        ];
        for changed in live {
            assert!(!config.requires_restart(&changed), "{changed:?}");
        }
        for changed in hardware {
            assert!(config.requires_restart(&changed), "{changed:?}");
        }
    }

    #[test]
    fn settings_are_read_and_checked() {
        let config = Config::from_json(r#"{"display": "raw", "m4": false, "unidos": false}"#).unwrap();
        assert!(config.display().unwrap().is_raw());
        let mb = config.build(None, &sd()).unwrap();
        assert!(!mb.has_expansion::<amsrust::m4::M4Board>());
        assert_eq!(amsrust::autorun::boot_seconds(&mb), amsrust::autorun::AUTORUN_AT_SECONDS);
        assert!(Config::from_json(r#"{"colour": true}"#).is_err());
        assert!(Config::from_json(r#"{"display": "tv"}"#).unwrap().display().is_err());
        let plus = config.build(Some(CpcModel::Cpc6128Plus), &sd()).unwrap();
        assert!(plus.model().is_plus());
    }
}
