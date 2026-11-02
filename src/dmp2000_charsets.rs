//! The DMP2000's national character sets (its manual, chapter 2): twelve codes change with the
//! country, ESC R n (or the DIP switches) choosing it.
//!
//! The printer's own glyphs are not available: the national characters are drawn from the CPC's
//! 8x8 font (a letter and its accent, or a few small bitmaps).

/// The codes the countries change.
pub const NATIONAL_CODES: [u8; 12] = [0x23, 0x24, 0x40, 0x5b, 0x5c, 0x5d, 0x5e, 0x60, 0x7b, 0x7c, 0x7d, 0x7e];

/// The country sets, as ESC R n numbers them: USA, France, Germany, UK, Denmark, Sweden,
/// Italy, Spain, Japan. What stands at each of the [`NATIONAL_CODES`].
const SETS: [&str; 9] = [
    "#$@[\\]^`{|}~",
    "#$à°ç§^`éùè¨",
    "#$§ÄÖÜ^`äöüß",
    "£$@[\\]^`{|}~",
    "#$@ÆØÅ^`æøå~",
    "#¤ÉÄÖÅÜéäöåü",
    "#$@°\\é^ùàòèì",
    "₧$@¡Ñ¿^`¨ñ}~",
    "#$@[¥]^`{|}~"
];

/// The character a country puts at `code`.
pub fn national_char(set: u8, code: u8) -> Option<char> {
    let index = NATIONAL_CODES.iter().position(|&national| national == code)?;
    SETS.get(usize::from(set))?.chars().nth(index)
}

/// Eight rows from a picture (`#` a dot).
fn art(rows: [&str; 8]) -> [u8; 8] {
    let mut glyph = [0u8; 8];
    for (row, picture) in glyph.iter_mut().zip(rows) {
        for (column, c) in picture.chars().enumerate() {
            if c == '#' {
                *row |= 0x80 >> column;
            }
        }
    }
    glyph
}

/// Drops a glyph one row, to make room for an accent over a capital.
fn lower(glyph: [u8; 8]) -> [u8; 8] {
    let mut lowered = [0u8; 8];
    lowered[1..].copy_from_slice(&glyph[..7]);
    lowered
}

/// Where the printer's EPROM keeps a national character (codes 0-31: the font of its near
/// letter quality has the ASCII ones where ASCII has them).
pub fn eprom_code(character: char) -> Option<u8> {
    const CODES: &str = "àèùòì°£¡¿Ññ¤₧Ååç§ßÆæØø¨ÄÖÜäöüÉé¥";
    CODES.chars().position(|c| c == character).map(|code| code as u8)
}

/// The printer's `^`: the CPC's font has an arrow at that code.
pub fn caret() -> [u8; 8] {
    art(["..##....", ".#..#...", "#....#..", "........", "........", "........", "........", "........"])
}

/// The glyph of a national character, from the CPC's font (`font`: 256 glyphs, a row a byte).
/// `None` for what the font has itself.
pub fn glyph(character: char, font: &[[u8; 8]]) -> Option<[u8; 8]> {
    let of = |c: char| font[usize::from(c as u8)];
    // accents over the room a small letter leaves (two rows), and over a capital (one)
    let small = |base: char, accent: [u8; 2]| {
        let mut glyph = of(base);
        if base == 'i' {
            glyph[0] = 0;
            glyph[1] = 0;
        }
        glyph[0] |= accent[0];
        glyph[1] |= accent[1];
        glyph
    };
    let capital = |base: char, accent: u8| {
        let mut glyph = lower(of(base));
        glyph[0] |= accent;
        glyph
    };
    const ACUTE: [u8; 2] = [0x08, 0x10];
    const GRAVE: [u8; 2] = [0x10, 0x08];
    const DIAERESIS: [u8; 2] = [0x00, 0x28];
    const RING: [u8; 2] = [0x18, 0x18];
    const TILDE: [u8; 2] = [0x28, 0x50];

    Some(match character {
        '£' => font[0xa3],
        'à' => small('a', GRAVE),
        'é' => small('e', ACUTE),
        'è' => small('e', GRAVE),
        'ì' => small('i', GRAVE),
        'ò' => small('o', GRAVE),
        'ù' => small('u', GRAVE),
        'ä' => small('a', DIAERESIS),
        'ö' => small('o', DIAERESIS),
        'ü' => small('u', DIAERESIS),
        'å' => small('a', RING),
        'ñ' => small('n', TILDE),
        'ç' => {
            let mut glyph = of('c');
            glyph[7] |= 0x10;
            glyph
        },
        'Ä' => capital('A', 0x28),
        'Ö' => capital('O', 0x28),
        'Ü' => capital('U', 0x28),
        'Å' => capital('A', 0x18),
        'É' => capital('E', 0x08),
        'Ñ' => capital('N', 0x28),
        'ø' => {
            let mut glyph = of('o');
            for (row, bits) in [(2, 0x02), (3, 0x04), (4, 0x08), (5, 0x10), (6, 0x20)] {
                glyph[row] |= bits;
            }
            glyph
        },
        'Ø' => {
            let mut glyph = lower(of('O'));
            for (row, bits) in [(1, 0x02), (2, 0x04), (3, 0x08), (4, 0x10), (5, 0x20), (6, 0x40)] {
                glyph[row] |= bits;
            }
            glyph
        },
        '¡' | '¿' => {
            let mut glyph = of(if character == '¡' { '!' } else { '?' });
            glyph[..7].reverse();
            glyph
        },
        '¥' => {
            let mut glyph = of('Y');
            glyph[3] |= 0x7c;
            glyph[4] |= 0x7c;
            glyph
        },
        '₧' => of('P'),
        '¨' => {
            let mut glyph = [0u8; 8];
            glyph[0] = 0x28;
            glyph
        },
        '°' => art(["..##....", ".#..#...", ".#..#...", "..##....", "........", "........", "........", "........"]),
        '§' => art([".####...", "##......", ".###....", "##.##...", ".###....", "....##..", "####....", "........"]),
        '¤' => art(["........", "#.....#.", ".#####..", ".#...#..", ".#...#..", ".#####..", "#.....#.", "........"]),
        'ß' => art([".###....", "##.##...", "##.##...", "##.##...", "##..##..", "##..##..", "##.##...", "##......"]),
        'æ' => art(["........", "........", ".##.##..", "...#..#.", ".####.##", "#..#.#..", ".##.###.", "........"]),
        'Æ' => art(["..#####.", ".##.#...", "##..#...", "##.###..", "######..", "##..#...", "##..####", "........"]),
        _ => return None
    })
}

#[cfg(test)]
mod test {
    use super::*;

    fn font() -> Vec<[u8; 8]> {
        (0..256usize)
            .map(|code| {
                let mut glyph = [0u8; 8];
                glyph.copy_from_slice(&crate::roms::OS6128[0x3800 + code * 8..0x3800 + code * 8 + 8]);
                glyph
            })
            .collect()
    }

    #[test]
    fn the_sets_have_twelve_characters_each() {
        for set in SETS {
            assert_eq!(set.chars().count(), 12, "{set}");
        }
    }

    #[test]
    fn the_uk_has_a_pound_where_the_usa_has_a_hash() {
        assert_eq!(national_char(0, 0x23), Some('#'));
        assert_eq!(national_char(3, 0x23), Some('£'));
        assert_eq!(national_char(1, 0x7b), Some('é'));
        assert_eq!(national_char(2, 0x7e), Some('ß'));
        assert_eq!(national_char(3, b'A'), None);
    }

    #[test]
    fn every_national_character_of_the_sets_has_a_glyph_or_is_in_the_font() {
        let font = font();
        for set in SETS {
            for character in set.chars() {
                if character.is_ascii() {
                    continue;
                }
                let glyph = glyph(character, &font).unwrap_or_else(|| panic!("no glyph for {character}"));
                assert!(glyph.iter().any(|&row| row != 0), "{character} is blank");
            }
        }
    }

    #[test]
    fn the_eprom_has_every_national_character() {
        for set in SETS {
            for character in set.chars().filter(|c| !c.is_ascii()) {
                assert!(eprom_code(character).is_some(), "{character}");
            }
        }
        assert_eq!(eprom_code('à'), Some(0));
        assert_eq!(eprom_code('£'), Some(6));
        assert_eq!(eprom_code('¥'), Some(31));
        assert_eq!(eprom_code('a'), None);
    }

    #[test]
    fn an_accent_makes_a_glyph_of_its_own() {
        let font = font();
        let a = font[usize::from(b'a')];
        for accented in ['à', 'ä', 'å'] {
            assert_ne!(glyph(accented, &font), Some(a));
        }
        assert_ne!(glyph('é', &font), glyph('è', &font));
    }
}
