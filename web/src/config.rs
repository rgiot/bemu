//! The machine and display an embedding page asks for, as a JavaScript object.

use amsrust::crtc::CrtcType;
use amsrust::display::DisplayMode;
use amsrust::ga::GateArrayType;
use amsrust::motherboard::{CpcModel, MotherboardBuilder};
use amsrust::ppi::Brand;
use serde::Deserialize;

/// `{ model, crtc, gateArray, ramKb, discInterface, playcity, digiblaster, willyOpl3, language, brand, display, curvature, blackLevel }`, each optional:
/// a 6128 with its own disc interface, CRTC 0, Gate Array 40010, raw display by default.
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Config {
    /// "464", "664" or "6128".
    pub model: Option<String>,
    /// 0 to 4.
    pub crtc: Option<u8>,
    /// "40007", "40008" or "40010".
    pub gate_array: Option<String>,
    /// All the RAM, in KB: 64 to 576 by 64 (the standard extra banks), or 4160 (a 4MB expansion).
    pub ram_kb: Option<usize>,
    /// A disc interface fitted (a DDI-1 on a 464) or removed; the model's own if unset.
    pub disc_interface: Option<bool>,
    /// A PlayCity plugged (two more AYs and a CTC).
    pub playcity: Option<bool>,
    /// A Digiblaster plugged (a DAC on the printer port).
    pub digiblaster: Option<bool>,
    /// Willy with an OPL3LPT plugged: true (at &FEBC) or its base ("&FEAC"...).
    pub willy_opl3: Option<WillyOpl3>,
    /// "en" (the default) or "fr": the French 6128's ROMs, AZERTY keyboard.
    pub language: Option<String>,
    /// The name of the boot banner: "amstrad", "schneider", "awa", "orion", "saisho", "solavox",
    /// "triumph" or "isp".
    pub brand: Option<String>,
    /// "raw", or a CRT: "ctm", "trinitron", "gt65", "atari".
    pub display: Option<String>,
    /// How curved a CRT's glass is, times its profile's bulge: 0 (flat) to 4, 1 by default.
    pub curvature: Option<f32>,
    /// The level a black ink is drawn at, 0-255 (8 by default: a tube's dark grey).
    pub black_level: Option<u8>
}

/// `willyOpl3`: `true` (at &FEBC) or the base its jumper sets (`"&FEAC"`...).
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum WillyOpl3 {
    Plugged(bool),
    At(String)
}

impl Config {
    pub fn from_js(value: wasm_bindgen::JsValue) -> Result<Self, String> {
        if value.is_undefined() || value.is_null() {
            return Ok(Self::default());
        }
        serde_wasm_bindgen::from_value(value).map_err(|e| format!("configuration: {e}"))
    }

    /// The machine (interrupts enabled, as the firmware expects).
    pub fn machine(&self) -> Result<MotherboardBuilder, String> {
        let mut builder = MotherboardBuilder::default();
        builder.enable_interruptions();
        if let Some(model) = &self.model {
            builder.set_model(model.parse::<CpcModel>()?);
        }
        if let Some(crtc) = self.crtc {
            builder.set_crtc(CrtcType::try_from(crtc).map_err(|_| format!("crtc must be 0-4, not {crtc}"))?);
        }
        if let Some(ga) = &self.gate_array {
            builder.set_gate_array_type(match ga.as_str() {
                "40010" => GateArrayType::Ga40010,
                "40007" | "40008" => GateArrayType::Ga40007_8,
                other => return Err(format!("gateArray must be 40007, 40008 or 40010, not {other:?}"))
            });
        }
        match self.ram_kb {
            None => {},
            Some(4160) => {
                builder.plug_large_ram_expansion(64);
            },
            Some(kb) if (64..=576).contains(&kb) && kb % 64 == 0 => {
                builder.set_extra_ram_pages(kb / 64 - 1);
            },
            Some(kb) => return Err(format!("ramKb must be 64, 128, ... 576 or 4160, not {kb}"))
        }
        if let Some(fitted) = self.disc_interface {
            builder.set_disc_interface(fitted);
        }
        if let Some(brand) = &self.brand {
            builder.set_brand(brand_of(brand)?);
        }
        if self.playcity == Some(true) {
            #[cfg(feature = "playcity")]
            builder.plug_playcity();
            #[cfg(not(feature = "playcity"))]
            return Err("playcity: this build has no PlayCity".into());
        }
        if self.digiblaster == Some(true) {
            #[cfg(feature = "digiblaster")]
            builder.plug_digiblaster();
            #[cfg(not(feature = "digiblaster"))]
            return Err("digiblaster: this build has no Digiblaster".into());
        }
        match &self.willy_opl3 {
            None | Some(WillyOpl3::Plugged(false)) => {},
            #[cfg(feature = "willy")]
            Some(WillyOpl3::Plugged(true)) => {
                builder.plug_willy_opl3(amsrust::willy::OPL3_BASE);
            },
            #[cfg(feature = "willy")]
            Some(WillyOpl3::At(base)) => {
                builder.plug_willy_opl3(amsrust::willy::parse_base(base).map_err(|e| format!("willyOpl3: {e}"))?);
            },
            #[cfg(not(feature = "willy"))]
            Some(_) => return Err("willyOpl3: this build has no Willy".into())
        }
        match self.language.as_deref() {
            None | Some("en") => {},
            Some("fr") if self.model.as_deref().is_none_or(|model| model == "6128") => {
                builder.set_keyboard_layout(amsrust::keyboard::KeyboardLayout::French);
            },
            Some("fr") => return Err("language fr: there are French ROMs for the 6128 only".to_owned()),
            Some(other) => return Err(format!("language must be en or fr, not {other:?}"))
        }
        Ok(builder)
    }

    pub fn display(&self) -> Result<DisplayMode, String> {
        self.display.as_deref().unwrap_or("raw").parse()
    }

    pub fn curvature(&self) -> Result<f32, String> {
        match self.curvature {
            None => Ok(0.0),
            Some(c) if (0.0..=4.0).contains(&c) => Ok(c),
            Some(c) => Err(format!("curvature must be 0 to 4, not {c}"))
        }
    }

    pub fn black_level(&self) -> u8 {
        self.black_level.unwrap_or(8)
    }
}

fn brand_of(name: &str) -> Result<Brand, String> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "amstrad" => Brand::Amstrad,
        "schneider" => Brand::Schneider,
        "awa" => Brand::Awa,
        "orion" => Brand::Orion,
        "saisho" => Brand::Saisho,
        "solavox" => Brand::Solavox,
        "triumph" => Brand::Triumph,
        "isp" | "lsp" => Brand::Lsp,
        other => return Err(format!("unknown brand {other:?}"))
    })
}

#[cfg(test)]
mod test {
    use super::*;

    fn config(json: &str) -> Result<Config, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    #[test]
    fn a_configuration_builds_its_machine() {
        let c = config(r#"{"model": "464", "crtc": 2, "gateArray": "40007", "ramKb": 576, "discInterface": true,
                           "brand": "Schneider", "display": "ctm", "blackLevel": 0}"#)
            .unwrap();
        let machine = c.machine().unwrap();
        assert_eq!(machine.crtc(), CrtcType::Crtc2);
        assert_eq!(machine.gate_array_type(), GateArrayType::Ga40007_8);
        assert!(!c.display().unwrap().is_raw());
        assert_eq!(c.black_level(), 0);
        let mb = machine.build();
        assert_eq!(mb.model(), CpcModel::Cpc464);
        assert!(mb.has_disc_interface());

        assert!(config("{}").unwrap().machine().is_ok(), "all defaults");
        assert!(config(r#"{"crtc": 5}"#).unwrap().machine().is_err());
        assert!(config(r#"{"ramKb": 100}"#).unwrap().machine().is_err());
        assert!(config(r#"{"model": "6129"}"#).unwrap().machine().is_err());
        assert!(config(r#"{"display": "tv"}"#).unwrap().display().is_err());
        assert!(config(r#"{"colour": true}"#).is_err(), "unknown fields are refused");
        let french = config(r#"{"language": "fr"}"#).unwrap().machine().unwrap().build();
        assert_eq!(french.keyboard_layout(), amsrust::keyboard::KeyboardLayout::French);
        assert!(config(r#"{"language": "fr", "model": "464"}"#).unwrap().machine().is_err());
        assert!(config(r#"{"language": "de"}"#).unwrap().machine().is_err());
    }

    #[cfg(feature = "willy")]
    #[test]
    fn willy_with_an_opl3_is_plugged_at_its_base() {
        let plugged = |json: &str| {
            let mb = config(json).unwrap().machine().unwrap().build();
            mb.expansion_port().and_then(|port| port.device::<amsrust::willy::Opl3Lpt>()).map(|card| card.base())
        };
        assert_eq!(plugged(r#"{"willyOpl3": true}"#), Some(0xfebc));
        assert_eq!(plugged(r#"{"willyOpl3": "&FEAC"}"#), Some(0xfeac));
        assert_eq!(plugged(r#"{"willyOpl3": false}"#), None);
        assert!(config(r#"{"willyOpl3": "&FE00"}"#).unwrap().machine().is_err());
    }
}
