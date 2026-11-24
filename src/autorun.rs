//! Autorun: what to type for a disc, a tape or a cartridge to start by itself, as a user would
//! once the machine has booted (`RUN"NAME`, `|CPM`, `|TAPE` then `RUN"`).
//!
//! The text is meant for a [`crate::typist::Typist`] (CSL `key_output` escapes: `\(RET)`,
//! `\(WAIT n)`). The desktop's `--autorun` and the Android frontend use it.

use crate::disc_image::{AmsdosEntry, DiscImage};

/// When the typing starts, in seconds after a reset, on a stock machine: BASIC is ready.
pub const AUTORUN_AT_SECONDS: f64 = 3.0;

/// The same with UniDOS or the M4: their initialisation lasts until about 4 s (keys typed
/// before are lost), a second more for margin.
pub const AUTORUN_EXPANDED_AT_SECONDS: f64 = 5.0;

/// How long after a reset the autorun waits at most for the firmware to wait for a key (beyond
/// [`boot_seconds`]): a machine that never reaches BASIC's prompt still gets the text typed.
pub const AUTORUN_MARGIN_SECONDS: f64 = 2.0;

/// The typist of an autorun's `text` on `motherboard`, just reset: it types as soon as the firmware
/// waits for a key (BASIC's prompt), [`boot_seconds`] + [`AUTORUN_MARGIN_SECONDS`] at the latest.
pub fn typist(text: &str, motherboard: &crate::motherboard::Motherboard) -> crate::typist::Typist {
    crate::typist::Typist::when_waiting_for_a_key(
        text,
        boot_seconds(motherboard) + AUTORUN_MARGIN_SECONDS,
        motherboard.keyboard_layout(),
    )
}

/// When to start typing on `motherboard` once reset, when the firmware cannot be watched (a fixed
/// time: see [`typist`], which waits for the firmware instead): [`AUTORUN_EXPANDED_AT_SECONDS`] when it
/// has UniDOS (an upper ROM other than AMSDOS in slot 7) or the M4, else
/// [`AUTORUN_AT_SECONDS`].
pub fn boot_seconds(motherboard: &crate::motherboard::Motherboard) -> f64 {
    #[allow(unused_mut)]
    let mut expanded = motherboard
        .upper_rom(7)
        .is_some_and(|rom| !crate::roms::AMSDOS.starts_with(&rom[..rom.len().min(0x4000)]));
    #[cfg(feature = "m4")]
    {
        expanded |= motherboard.has_expansion::<crate::m4::M4Board>();
    }
    if expanded {
        AUTORUN_EXPANDED_AT_SECONDS
    } else {
        AUTORUN_AT_SECONDS
    }
}

/// The kinds of media a frontend can be given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// A DSK / EDSK or an HFE image, or an archive (zip, 7z, tar.gz) holding one.
    Disc,
    /// A CDT / TZX tape.
    Tape,
    /// A CPC+ / GX4000 cartridge (CPR).
    Cartridge,
    /// A SNA snapshot.
    Snapshot,
}

/// What `name` (a file name) and `bytes` are: by their signature, else by the extension.
pub fn media_kind(name: &str, bytes: &[u8]) -> Option<MediaKind> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    if starts(b"MV - CPC") || starts(b"EXTENDED") || starts(b"HXCPICFE") || starts(b"HXCHFEV3") {
        return Some(MediaKind::Disc);
    }
    if starts(b"ZXTape!") {
        return Some(MediaKind::Tape);
    }
    if starts(b"MV - SNA") {
        return Some(MediaKind::Snapshot);
    }
    if starts(b"RIFF") && bytes.get(8..12) == Some(b"AMS!") {
        return Some(MediaKind::Cartridge);
    }
    let lower = name.to_ascii_lowercase();
    let extension = lower.rsplit('.').next().unwrap_or("");
    match extension {
        "dsk" | "hfe" => Some(MediaKind::Disc),
        "cdt" | "tzx" => Some(MediaKind::Tape),
        "cpr" => Some(MediaKind::Cartridge),
        "sna" => Some(MediaKind::Snapshot),
        // an archive: its disc, else its cartridge
        "zip" | "7z" | "gz" | "tgz" => {
            let discs = crate::disc_image::archive_disc_entries(bytes);
            if discs.is_empty() && crate::asic::Cartridge::from_bytes(bytes, name, None).is_ok() {
                Some(MediaKind::Cartridge)
            } else {
                Some(MediaKind::Disc)
            }
        }
        _ => None,
    }
}

/// The program to `RUN` when nobody names one: an obvious menu / loader name, then the biggest
/// `.BIN`, then the biggest `.BAS`, then a lone file without extension, then whatever real file
/// sorts first. Only entries [`AmsdosEntry::is_plausible_filename`] accepts (real files, not
/// CATART), and of user 0 (the one `RUN"` reads).
pub fn pick_program(catalog: &[AmsdosEntry]) -> Option<&AmsdosEntry> {
    let real: Vec<&AmsdosEntry> = catalog
        .iter()
        .filter(|e| e.is_plausible_filename() && e.user == 0)
        .collect();
    if let [only] = real.as_slice() {
        return Some(only);
    }
    let by_name = |wanted: &str| {
        real.iter()
            .find(|e| e.name.eq_ignore_ascii_case(wanted))
            .copied()
    };
    let contains = |needle: &str| {
        real.iter()
            .filter(|e| e.name.to_ascii_uppercase().contains(needle))
            .max_by_key(|e| e.records)
            .copied()
    };
    let biggest_with_extension = |extension: &str| {
        real.iter()
            .filter(|e| e.extension.eq_ignore_ascii_case(extension))
            .max_by_key(|e| e.records)
            .copied()
    };
    by_name("DISC")
        .or_else(|| by_name("RUN"))
        .or_else(|| contains("MENU"))
        .or_else(|| contains("LOADER"))
        .or_else(|| contains("START"))
        .or_else(|| contains("INTRO"))
        .or_else(|| biggest_with_extension("BAS"))
        .or_else(|| biggest_with_extension("BIN"))
        .or_else(|| biggest_with_extension(""))
        .or_else(|| real.iter().min_by_key(|e| e.filename()).copied())
}

/// Whether the disc is in the system (CP/M) format: its sectors numbered from &41.
fn is_system_disc(disc: &DiscImage) -> bool {
    disc.track(0, 0)
        .and_then(|track| track.sectors.iter().map(|s| s.r).min())
        .is_some_and(|first| first == 0x41)
}

/// What to type to start `disc`: `RUN"NAME` for its program, or `|CPM` for a system disc
/// without one. `None` when nothing on it looks runnable.
pub fn disc_command(disc: &DiscImage) -> Option<String> {
    let catalog = disc.amsdos_catalog();
    match pick_program(&catalog) {
        Some(entry) => Some(format!("RUN\"{}\\(RET)", entry.filename())),
        None if is_system_disc(disc) => Some("|CPM\\(RET)".to_owned()),
        None => None,
    }
}

/// What to type to load a tape's first program: `|TAPE` first on a machine with a disc ROM,
/// then `RUN"`, and a key for "Press PLAY then any key".
pub fn tape_command(has_disc_rom: bool) -> String {
    let tape = if has_disc_rom { "|TAPE\\(RET)" } else { "" };
    format!("{tape}RUN\"\\(RET)\\(WAIT 1)\\(RET)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, extension: &str, records: u8) -> AmsdosEntry {
        AmsdosEntry {
            user: 0,
            name: name.to_owned(),
            extension: extension.to_owned(),
            hidden: false,
            records,
        }
    }

    #[test]
    fn a_lone_program_is_run() {
        let catalog = [entry("GAME", "BIN", 10)];
        assert_eq!(pick_program(&catalog).unwrap().filename(), "GAME.BIN");
    }

    #[test]
    fn a_loader_is_preferred_to_the_data() {
        let catalog = [
            entry("LEVEL1", "DAT", 120),
            entry("GAME", "BIN", 100),
            entry("LOADER", "BAS", 2),
        ];
        assert_eq!(pick_program(&catalog).unwrap().filename(), "LOADER.BAS");
        let catalog = [entry("GAME", "BIN", 100), entry("GAME", "BAS", 2)];
        assert_eq!(pick_program(&catalog).unwrap().filename(), "GAME.BAS");
    }

    #[test]
    fn catart_entries_are_not_programs() {
        let catalog = [entry("\x1f\x01\x02", "\x0f", 0), entry("DEMO", "", 80)];
        assert_eq!(pick_program(&catalog).unwrap().filename(), "DEMO");
        assert!(pick_program(&[entry("\x1c\x01", "", 0)]).is_none());
    }

    #[test]
    fn media_are_told_apart() {
        assert_eq!(media_kind("x.dsk", b""), Some(MediaKind::Disc));
        assert_eq!(media_kind("X.HFE", b""), Some(MediaKind::Disc));
        assert_eq!(media_kind("x", b"ZXTape!\x1a"), Some(MediaKind::Tape));
        assert_eq!(media_kind("game.CPR", b""), Some(MediaKind::Cartridge));
        assert_eq!(media_kind("x.sna", b""), Some(MediaKind::Snapshot));
        assert_eq!(media_kind("x.txt", b"hello"), None);
    }

    #[test]
    fn a_tape_is_redirected_on_a_disc_machine() {
        assert!(tape_command(true).starts_with("|TAPE"));
        assert!(tape_command(false).starts_with("RUN\""));
    }
}
