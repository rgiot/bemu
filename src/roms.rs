//! The ROMs embedded in the executable, named, so one file is enough to run everything:
//! `inner://NAME` stands for one of them wherever a ROM file is asked for.

/// A 16K ROM.
pub type RomImage = [u8; 0x4000];

pub static OS464: &RomImage = include_bytes!("../roms/os464.rom");
pub static OS664: &RomImage = include_bytes!("../roms/os664.rom");
pub static OS6128: &RomImage = include_bytes!("../roms/os6128.rom");
pub static OS6128_AZERTY: &RomImage = include_bytes!("../roms/os6128_azerty.rom");
pub static BASIC464: &RomImage = include_bytes!("../roms/basic464.rom");
pub static BASIC664: &RomImage = include_bytes!("../roms/basic664.rom");
pub static BASIC6128: &RomImage = include_bytes!("../roms/basic6128.rom");
pub static BASIC6128_AZERTY: &RomImage = include_bytes!("../roms/basic6128_azerty.rom");
pub static AMSDOS: &RomImage = include_bytes!("../roms/amsdos.rom");
/// The M4 board's ROM (Duke's M4ROM v2.0.8).
#[cfg(feature = "m4")]
pub static M4: &RomImage = include_bytes!("../roms/m4rom.rom");
/// UniDOS 1.51 (Futurs') and its DOS nodes.
#[cfg(feature = "unidos")]
pub static UNIDOS: &RomImage = include_bytes!("../roms/unidos.rom");
#[cfg(feature = "unidos")]
pub static UNIDOS_NOVA: &RomImage = include_bytes!("../roms/unidos-nova.rom");
#[cfg(feature = "unidos")]
pub static UNIDOS_ALBIREO: &RomImage = include_bytes!("../roms/unidos-albireo.rom");
#[cfg(feature = "unidos")]
pub static UNIDOS_M4: &RomImage = include_bytes!("../roms/unidos-m4.rom");
#[cfg(feature = "unidos")]
pub static UNITOOLS: &RomImage = include_bytes!("../roms/unitools.rom");
/// Orgams (BDC/Madram & co), the assembler and its tools, beta J (2025-12-28).
#[cfg(feature = "orgams")]
pub static ORGAMS: &RomImage = include_bytes!("../roms/orgams.rom");
#[cfg(feature = "orgams")]
pub static ORGEXT: &RomImage = include_bytes!("../roms/orgext.rom");
#[cfg(feature = "orgams")]
pub static MONOGAMS: &RomImage = include_bytes!("../roms/monogams.rom");
#[cfg(feature = "orgams")]
pub static BRICBRAC: &RomImage = include_bytes!("../roms/bricbrac.rom");
#[cfg(feature = "orgams")]
pub static ORGASS: &RomImage = include_bytes!("../roms/orgass.rom");

/// An embedded ROM: its name, its bytes, what it is.
pub type Embedded = (&'static str, &'static RomImage, &'static str);

/// The embedded ROMs (those of the extensions built in).
pub fn embedded_roms() -> Vec<Embedded> {
    #[allow(unused_mut)]
    let mut roms: Vec<Embedded> = vec![
        ("os464", OS464, "CPC 464 firmware (lower ROM)"),
        ("os664", OS664, "CPC 664 firmware (lower ROM)"),
        ("os6128", OS6128, "CPC 6128 firmware (lower ROM)"),
        ("os6128_azerty", OS6128_AZERTY, "CPC 6128 French firmware (lower ROM)"),
        ("basic464", BASIC464, "BASIC 1.0"),
        ("basic664", BASIC664, "BASIC 1.1 (664)"),
        ("basic6128", BASIC6128, "BASIC 1.1"),
        ("basic6128_azerty", BASIC6128_AZERTY, "BASIC 1.1, French"),
        ("amsdos", AMSDOS, "AMSDOS"),
    ];
    #[cfg(feature = "m4")]
    roms.push(("m4", M4, "the M4 board's ROM v2.0.8"));
    #[cfg(feature = "unidos")]
    roms.extend([
        ("unidos", UNIDOS, "UniDOS 1.51"),
        ("unidos-nova", UNIDOS_NOVA, "UniDOS node for the Nova"),
        ("unidos-albireo", UNIDOS_ALBIREO, "UniDOS node for the Albireo"),
        ("unidos-m4", UNIDOS_M4, "UniDOS node for the M4"),
        ("unitools", UNITOOLS, "UniDOS node UniTools"),
    ]);
    #[cfg(feature = "orgams")]
    roms.extend([
        ("orgams", ORGAMS, "Orgams, its base (a background ROM: slot 1-15)"),
        ("orgext", ORGEXT, "Orgams, its extension"),
        ("monogams", MONOGAMS, "Orgams, its monitor"),
        ("bricbrac", BRICBRAC, "Orgams, BricBrac (|BURN when in slot 1-15)"),
        ("orgass", ORGASS, "Orgams, Orgass"),
    ]);
    roms
}

const SCHEME: &str = "inner://";

/// An embedded ROM by name (`.rom` optional, case ignored).
pub fn embedded(name: &str) -> Option<&'static RomImage> {
    let name = name.strip_suffix(".rom").or_else(|| name.strip_suffix(".ROM")).unwrap_or(name);
    embedded_roms().into_iter().find(|(known, _, _)| known.eq_ignore_ascii_case(name)).map(|(_, rom, _)| rom)
}

/// The names, one per line, with what each is.
pub fn listing() -> String {
    embedded_roms().iter().map(|(name, _, what)| format!("{SCHEME}{name:<16} {what}\n")).collect()
}

/// A ROM: `inner://NAME`, an embedded one, or a file.
pub fn load_rom(spec: &str) -> Result<Vec<u8>, String> {
    match spec.strip_prefix(SCHEME) {
        Some(name) => embedded(name)
            .map(|rom| rom.to_vec())
            .ok_or_else(|| format!("{spec}: no such embedded ROM; there are:\n{}", listing())),
        None => std::fs::read(spec).map_err(|e| format!("{spec}: {e}"))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    #[cfg(feature = "unidos")]
    fn embedded_roms_by_name_or_files() {
        let unidos = load_rom("inner://unidos").unwrap();
        assert_eq!((unidos.len(), unidos[0]), (0x4000, 1), "a background ROM");
        assert_eq!(load_rom("inner://UNIDOS-NOVA.ROM").unwrap()[0], 2, "a node: an extension ROM");
        let error = load_rom("inner://nothing").unwrap_err();
        assert!(error.contains("inner://unitools"), "{error}");
        let path = std::env::temp_dir().join(format!("bemu-rom-{}.rom", std::process::id()));
        std::fs::write(&path, [1, 2, 3]).unwrap();
        assert_eq!(load_rom(path.to_str().unwrap()).unwrap(), vec![1, 2, 3]);
        let _ = std::fs::remove_file(path);
    }
}
