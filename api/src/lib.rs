//! The types of bemu's debug API (`--debug-server`): what a request takes and a reply gives.
//!
//! The server parses its requests into these types and builds its replies from them, and its
//! OpenAPI description (`GET /api/doc?format=openapi`) is generated from their JSON schemas: what
//! is documented is what is parsed. A client (a debugger) can use the same types.
//!
//! Conventions shared with AMSpiriT's API, which bemu serves:
//! - an integer is a JSON number or a string, decimal or hexadecimal with `0x` ([`Int`]);
//! - a JSON body's field may be nested at any depth (`{"a":1}` and `{"x":{"a":1}}` both carry
//!   `a`): the server lifts them before parsing;
//! - an error answers 400 with an [`ErrorReply`].

use std::borrow::Cow;

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What the server offers beyond the AMSpiriT API, listed in [`Ping::features`].
pub const FEATURES: &[&str] = &[
    // `/api/step?mode=over|out|frame`
    "step_modes",
    // `/api/run_to`
    "run_to",
    // `/api/z80_bp?add=1`, `DELETE /api/z80_bp`
    "z80_bp_edit",
    // a `reason` and the `cmd_seq` in the stop and pause events
    "stop_reason",
    // `POST /api/z80` takes `HL`, `BC2`...
    "register_pairs",
    // a raster breakpoint's `name`, and its `index` in its stop event
    "raster_names",
    // `GET /api/doc?format=openapi` generated from these types
    "openapi",
];

// ------------------------------------------------------------------ values

/// An integer, as a JSON number or as a string: decimal, or hexadecimal with `0x`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Int(pub i64);

impl Int {
    /// `text` as the API reads an integer: decimal, or hexadecimal with `0x`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            Some(hex) => i64::from_str_radix(hex, 16).ok(),
            None => text.parse().ok(),
        }
        .map(Int)
    }

    /// The value, if it is within `min..=max`.
    pub fn within(self, min: i64, max: i64) -> Option<i64> {
        (min..=max).contains(&self.0).then_some(self.0)
    }
}

impl From<i64> for Int {
    fn from(value: i64) -> Self {
        Int(value)
    }
}

impl Serialize for Int {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.0)
    }
}

impl<'de> Deserialize<'de> for Int {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = Int;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an integer: a number, or a string (decimal, or hexadecimal with 0x)")
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Int, E> {
                Ok(Int(v))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Int, E> {
                i64::try_from(v).map(Int).map_err(|_| E::custom("integer too large"))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Int, E> {
                if v.fract() == 0.0 && v.abs() < 9.0e15 {
                    Ok(Int(v as i64))
                } else {
                    Err(E::invalid_type(serde::de::Unexpected::Float(v), &self))
                }
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Int, E> {
                Int::parse(v).ok_or_else(|| E::invalid_value(serde::de::Unexpected::Str(v), &self))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

impl JsonSchema for Int {
    fn schema_name() -> Cow<'static, str> {
        "Int".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "An integer: a JSON number, or a string, decimal or hexadecimal with 0x.",
            "oneOf": [
                { "type": "integer" },
                { "type": "string", "pattern": "^\\s*(0[xX][0-9a-fA-F]+|-?[0-9]+)\\s*$" }
            ]
        })
    }
}

// ------------------------------------------------------------------ replies shared by many

/// An error: 400 for a request the server refuses, 404 for an unknown path, 405 for a method a
/// path does not take, 503 when the emulation did not answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorReply {
    /// What is wrong.
    pub error: String,
    /// The field or parameter at fault, when one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

/// A command accepted. Its effects happen as the emulation runs: the events they cause carry its
/// `cmd_seq`, and `GET /api/ping`'s `emu.applied_cmd_seq` reaches it once it is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommandAck {
    /// Always true.
    pub ok: bool,
    /// The command's number, counting from 1.
    pub cmd_seq: u64,
}

impl CommandAck {
    pub fn new(cmd_seq: u64) -> Self {
        CommandAck { ok: true, cmd_seq }
    }
}

// ------------------------------------------------------------------ GET /api/ping

/// `GET /api/ping`: the server is there, and the emulator's state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ping {
    /// Always true.
    pub ok: bool,
    pub emu: EmuState,
    /// `bemu`: which emulator serves this AMSpiriT-compatible API.
    pub frontend: String,
    /// bemu's version.
    pub version: String,
    /// What it offers beyond the AMSpiriT API (see [`FEATURES`]).
    pub features: Vec<String>,
}

/// The emulator, as `/api/ping` and `/api/state` describe it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EmuState {
    /// The model: 464, 664, 6128, 464plus, 6128plus, gx4000.
    pub cpc_model: String,
    /// The CRTC type, 0 to 4.
    pub crtc_type: u8,
    /// Whether the machine is paused.
    pub paused: bool,
    /// Frames emulated.
    pub frames: u64,
    /// Frames per second, measured over the last second.
    pub fps: f64,
    /// NOPs (microseconds) emulated.
    pub ticks: u64,
    /// The last command applied (see [`CommandAck::cmd_seq`]).
    pub applied_cmd_seq: u64,
    /// The number of memory changes made through the API.
    pub ram_apply_seq: u64,
    /// Whether text is being typed (`/api/keytype`).
    pub autotyping: bool,
    /// Characters left to type.
    pub autotype_remaining: u64,
}

// ------------------------------------------------------------------ /api/z80

/// `GET /api/z80`: the Z80's registers. The alternate set is `A2`..`L2`.
#[allow(non_snake_case)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Z80Registers {
    pub PC: u16,
    pub SP: u16,
    pub IX: u16,
    pub IY: u16,
    pub A: u8,
    pub F: u8,
    pub B: u8,
    pub C: u8,
    pub D: u8,
    pub E: u8,
    pub H: u8,
    pub L: u8,
    pub A2: u8,
    pub F2: u8,
    pub B2: u8,
    pub C2: u8,
    pub D2: u8,
    pub E2: u8,
    pub H2: u8,
    pub L2: u8,
    pub I: u8,
    pub R: u8,
    /// 0 or 1.
    pub IFF1: u8,
    /// 0 or 1.
    pub IFF2: u8,
    /// The interrupt mode, 0 to 2.
    pub IM: u8,
    /// NOPs emulated.
    pub ticks: u64,
    /// WZ (MEMPTR).
    #[serde(default)]
    pub WZ: u16,
    /// The flags set, by name, bits 7 to 0 (`SZYHXPNC`), a dash for a clear one.
    #[serde(default)]
    pub flags: String,
    /// On a HALT, waiting for an interrupt.
    #[serde(default)]
    pub halted: bool,
}

/// `POST /api/z80`: the registers to set, by name; any left out are kept. All are checked
/// before any is changed. The pairs (`AF`..`HL`, `AF2`..`HL2`) set their two halves.
#[allow(non_snake_case)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetZ80 {
    /// 0 to 0xFFFF, like `SP`, `IX`, `IY` and the pairs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub PC: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub SP: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub IX: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub IY: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub AF: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub BC: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub DE: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub HL: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub AF2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub BC2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub DE2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub HL2: Option<Int>,
    /// 0 to 0xFF, like every 8-bit register.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub A: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub F: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub B: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub C: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub D: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub E: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub H: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub L: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub A2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub F2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub B2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub C2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub D2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub E2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub H2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub L2: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub I: Option<Int>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub R: Option<Int>,
    /// 0 or 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub IFF1: Option<Int>,
    /// 0 or 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub IFF2: Option<Int>,
    /// The interrupt mode, 0 to 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub IM: Option<Int>,
}

// ------------------------------------------------------------------ POST /api/step

/// How far `POST /api/step` runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StepMode {
    /// One instruction (a `z80_bp` event, reason `step`).
    #[default]
    In,
    /// The instruction, a `CALL`, `RST`, `DJNZ` or block loop run whole (reason `run_to`).
    Over,
    /// Until the routine returns: the stack above where it is (reason `step_out`).
    Out,
    /// To the end of the frame (a `pause` event).
    Frame,
}

/// `POST /api/step`'s query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StepQuery {
    /// How far to run: `in` (default), `over`, `out` or `frame`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<StepMode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_int_is_a_number_or_a_string() {
        let parse = |json: &str| serde_json::from_str::<Int>(json).map(|i| i.0).ok();
        assert_eq!(parse("16384"), Some(16384));
        assert_eq!(parse(r#""0x4000""#), Some(0x4000));
        assert_eq!(parse(r#"" 12 ""#), Some(12));
        assert_eq!(parse("2.0"), Some(2));
        assert_eq!(parse("2.5"), None);
        assert_eq!(parse(r#""&4000""#), None);
        assert_eq!(serde_json::to_string(&Int(7)).unwrap(), "7");
    }

    #[test]
    fn a_step_mode_is_its_name() {
        let query: StepQuery = serde_json::from_str(r#"{"mode":"over"}"#).unwrap();
        assert_eq!(query.mode, Some(StepMode::Over));
        assert!(serde_json::from_str::<StepQuery>(r#"{"mode":"sideways"}"#).is_err());
    }

    #[test]
    fn set_z80_takes_registers_by_name() {
        let set: SetZ80 = serde_json::from_str(r#"{"HL":"0x1234","A":7,"unknown":1}"#).unwrap();
        assert_eq!(set.HL, Some(Int(0x1234)));
        assert_eq!(set.A, Some(Int(7)));
        assert_eq!(set.PC, None);
    }
}

// ------------------------------------------------------------------ /api/psg

/// A PSG tone channel, as a musician reads it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PsgChannel {
    /// R0-R5: 12 bits.
    pub period: u16,
    pub frequency_hz: f64,
    /// The nearest note (`A4`), how far from it in cents, its MIDI number: none out of hearing.
    pub note: Option<String>,
    pub cents: Option<i32>,
    pub midi: Option<i32>,
    /// Its tone, and the noise, mixed in.
    pub tone: bool,
    pub noise: bool,
    /// Its volume, 0 to 15 (or the envelope's).
    pub volume: u8,
    pub envelope: bool,
}

impl PsgChannel {
    /// The note it sounds: its tone on, a volume.
    pub fn sounding(&self) -> Option<i32> {
        self.midi
            .filter(|_| self.tone && (self.envelope || self.volume != 0))
    }
}

/// `GET /api/psg`: AMSpiriT's fields, the 16 registers, and the channels read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Psg {
    pub period_a: u16,
    pub vol_a: u8,
    pub period_b: u16,
    pub vol_b: u8,
    pub period_c: u16,
    pub vol_c: u8,
    pub mixer: u8,
    pub noise: u8,
    pub env_period: u16,
    pub env_shape: u8,
    pub regs: Vec<u8>,
    pub selected_reg: Option<u8>,
    pub channels: Vec<PsgChannel>,
    pub noise_hz: f64,
    pub envelope_hz: f64,
    /// Port A reads (the keyboard), else writes.
    pub port_a_input: bool,
}

// ------------------------------------------------------------------ /api/fdc

/// A track in a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Unformatted,
    /// Sectors with several copies of their data.
    WeakSectors,
    /// The 9 sectors of 512 bytes of AMSDOS.
    Standard,
    NoSector,
    /// A protection, a more packed format...
    AnotherLayout,
}

impl TrackKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unformatted => "unformatted",
            Self::WeakSectors => "weak sectors",
            Self::Standard => "9 x 512",
            Self::NoSector => "no sector",
            Self::AnotherLayout => "another layout",
        }
    }
}

/// A sector of the track under the head.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiscSector {
    pub c: u8,
    pub h: u8,
    pub r: u8,
    pub n: u8,
    /// The size its N announces, and the data the image holds.
    pub size: usize,
    pub data_len: usize,
    /// What is special about it: weak (several copies), deleted, another track's ID, odd size.
    pub notes: Vec<String>,
}

/// A file of a disc's AMSDOS catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CatalogueEntry {
    /// `NAME.EXT`.
    pub name: String,
    pub user: u8,
    pub hidden: bool,
    pub kilobytes: u32,
}

/// The disc in a drive as the head sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Disc {
    /// `DSK` or `HFE`.
    pub image: String,
    /// The format of the track under the head: `Data`, `System`, `IBM`, else its sectors.
    pub format: String,
    pub tracks: u8,
    pub double_sided: bool,
    /// The side read: `A` or `B`.
    pub side: String,
    pub head_track: u8,
    /// Each track of the side read.
    pub track_kinds: Vec<TrackKind>,
    /// Each track's sector IDs (R), in their order on the track.
    pub track_sectors: Vec<Vec<u8>>,
    /// The track under the head, its sectors in the order they pass; the next one's index.
    pub sectors: Vec<DiscSector>,
    pub next_sector: Option<usize>,
    pub catalogue: Vec<CatalogueEntry>,
}

/// A drive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FdcDrive {
    pub connected: bool,
    /// `3` or `3.5` (inches).
    pub kind: String,
    pub disk: bool,
    /// `A` or `B`.
    pub side: String,
    pub track: u8,
    /// Its disc as the head sees it.
    pub disc: Option<Disc>,
}

/// `GET /api/fdc`: the status registers, the phase, the drives and their discs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Fdc {
    pub msr: u8,
    pub sr0: u8,
    pub sr1: u8,
    pub sr2: u8,
    pub motor: bool,
    pub phase: String,
    pub command: String,
    pub drives: Vec<FdcDrive>,
}

// ------------------------------------------------------------------ /api/asic

/// A DMA channel of the ASIC.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AsicDma {
    /// SAR: the next instruction's address.
    pub address: u16,
    /// PPR: a PAUSE tick is this + 1 HSYNCs.
    pub prescaler: u8,
    pub enabled: bool,
    /// PAUSE ticks left.
    pub pause: u16,
    /// REPEAT's counter.
    pub loop_count: u16,
    pub interrupt: bool,
}

/// A hardware sprite of the ASIC.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AsicSprite {
    pub x: u16,
    pub y: u16,
    /// 0 hides it; 1, 2, 3 make a pixel 1, 2, 4 wide or high.
    pub zoom_x: u8,
    pub zoom_y: u8,
    pub shown: bool,
    /// Its 16 x 16 pens, row by row (0: transparent, n: sprite colour n).
    pub pixels: Vec<u8>,
}

/// `GET /api/asic`: a CPC+'s ASIC (null on a CPC).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Asic {
    pub unlocked: bool,
    /// The 32 palette entries (12 bits, `&GRB`): pens 0-15, the border, sprite colours 1-15.
    pub palette: Vec<u16>,
    /// The line of the programmable raster interrupt (0: none).
    pub pri: u8,
    /// The line of the split screen (0: none).
    pub splt: u8,
    pub sscr: u8,
    pub ivr: u8,
    pub rmr2: u8,
    pub dma: Vec<AsicDma>,
    pub sprites: Vec<AsicSprite>,
}
