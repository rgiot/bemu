//! Symbols (labels) for the debuggers: names for addresses, from a snapshot's chunks (the
//! ACE `SYMB` chunk, the REMU chunk's `label` and `alias` entries, as basm and rasm write them)
//! or from a symbol file an assembler wrote.

use std::collections::BTreeMap;

/// How far after a label an address is still named after it (`label+offset`).
const NEAREST_RANGE: u16 = 0x100;

/// Names for addresses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SymbolTable {
    by_name: BTreeMap<String, u16>,
    /// The first name given to each address.
    by_address: BTreeMap<u16, String>
}

impl SymbolTable {
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Name `address` (a name given twice keeps its last address).
    pub fn insert(&mut self, name: &str, address: u16) {
        if name.is_empty() {
            return;
        }
        if let Some(previous) = self.by_name.insert(name.to_owned(), address) {
            if self.by_address.get(&previous).is_some_and(|n| n == name) {
                self.by_address.remove(&previous);
            }
        }
        self.by_address.entry(address).or_insert_with(|| name.to_owned());
    }

    pub fn extend(&mut self, other: &SymbolTable) {
        for (name, &address) in &other.by_name {
            self.insert(name, address);
        }
    }

    /// The symbols, by address.
    pub fn iter(&self) -> impl Iterator<Item = (&str, u16)> {
        let mut all: Vec<_> = self.by_name.iter().map(|(name, &address)| (name.as_str(), address)).collect();
        all.sort_by_key(|&(name, address)| (address, name));
        all.into_iter()
    }

    /// The address of `name` (exact, then ignoring case).
    pub fn address_of(&self, name: &str) -> Option<u16> {
        self.by_name.get(name).copied().or_else(|| {
            self.by_name.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, &address)| address)
        })
    }

    /// The name of exactly `address`.
    pub fn name_at(&self, address: u16) -> Option<&str> {
        self.by_address.get(&address).map(String::as_str)
    }

    /// `address` as `label` or `label+offset` (decimal), after the nearest label before it.
    pub fn describe(&self, address: u16) -> Option<String> {
        let (&at, name) = self.by_address.range(..=address).next_back()?;
        match address - at {
            0 => Some(name.clone()),
            offset if offset < NEAREST_RANGE => Some(format!("{name}+{offset}")),
            _ => None
        }
    }

    /// An address written by a person: a number (decimal, or hex as `0x`, `#`, `&`, `$` or an
    /// `h` suffix), a symbol, or a symbol plus or minus a number.
    pub fn resolve(&self, text: &str) -> Option<u16> {
        let text = text.trim();
        if let Some(value) = parse_number(text) {
            return Some(value);
        }
        if let Some(address) = self.address_of(text) {
            return Some(address);
        }
        let split = text.rfind(['+', '-'])?;
        let base = self.address_of(text[..split].trim())?;
        let offset = parse_number(text[split + 1..].trim())?;
        Some(if text.as_bytes()[split] == b'+' { base.wrapping_add(offset) } else { base.wrapping_sub(offset) })
    }

    /// The symbols of a snapshot chunk (`SYMB`, `REMU`); other chunks have none.
    pub fn add_snapshot_chunk(&mut self, code: &str, data: &[u8]) {
        match code {
            "SYMB" => self.add_ace_symbols(data),
            "REMU" => self.add_remu_labels(data),
            _ => {}
        }
    }

    /// ACE `SYMB`: records of a name length, the name, 6 bytes (memory map, type), the address
    /// high byte first.
    fn add_ace_symbols(&mut self, data: &[u8]) {
        let mut index = 0;
        while let Some(&length) = data.get(index) {
            let length = usize::from(length);
            let Some(record) = data.get(index..index + length + 9) else { break };
            if let Ok(name) = std::str::from_utf8(&record[1..=length]) {
                self.insert(name, u16::from_be_bytes([record[length + 7], record[length + 8]]));
            }
            index += length + 9;
        }
    }

    /// REMU: `label NAME ADDRESS BANK;` and `alias NAME VALUE;` entries.
    fn add_remu_labels(&mut self, data: &[u8]) {
        for entry in String::from_utf8_lossy(data).split(';') {
            let mut words = entry.split_whitespace();
            if let (Some("label" | "alias"), Some(name), Some(value)) = (words.next(), words.next(), words.next()) {
                if let Some(address) = parse_number(value) {
                    self.insert(name, address);
                }
            }
        }
    }

    /// A symbol file, one symbol a line, as the assemblers write them: `name equ value`,
    /// `name: equ value`, `name = value`, `name value` (basm, rasm) or `value name`. `;`
    /// starts a comment. The lines not understood are returned.
    pub fn parse_file(text: &str) -> (SymbolTable, Vec<String>) {
        let mut table = SymbolTable::default();
        let mut ignored = Vec::new();
        for line in text.lines() {
            let content = line.split(';').next().unwrap_or("").trim();
            if content.is_empty() {
                continue;
            }
            match parse_symbol_line(content) {
                Some((name, address)) => table.insert(name, address),
                None => ignored.push(line.to_owned())
            }
        }
        (table, ignored)
    }
}

fn is_identifier(word: &str) -> bool {
    let mut chars = word.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '.' || c == '@')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '$'))
}

/// A word naming a symbol (`name` or `name:`).
fn name_of(word: &str) -> Option<&str> {
    let word = word.strip_suffix(':').unwrap_or(word);
    is_identifier(word).then_some(word)
}

fn parse_symbol_line(line: &str) -> Option<(&str, u16)> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        [name, keyword, value, ..] if keyword.eq_ignore_ascii_case("equ") || *keyword == "=" => {
            Some((name_of(name)?, parse_number(value)?))
        },
        [name, value, ..] if name_of(name).is_some() && parse_number(value).is_some() => {
            Some((name_of(name)?, parse_number(value)?))
        },
        // `hhhh name`: the address first, hexadecimal even without a prefix
        [value, name, ..] => {
            let address = u16::from_str_radix(value, 16).ok().or_else(|| parse_number(value))?;
            Some((name_of(name)?, address))
        },
        _ => None
    }
}

/// A number as assemblers write them: decimal, or hexadecimal with `0x`, `#`, `&`, `$` or an `h`
/// suffix.
pub fn parse_number(text: &str) -> Option<u16> {
    let text = text.trim();
    let hex = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .or_else(|| text.strip_prefix('#'))
        .or_else(|| text.strip_prefix('&'))
        .or_else(|| text.strip_prefix('$'))
        .or_else(|| text.strip_suffix(['h', 'H']).filter(|t| t.starts_with(|c: char| c.is_ascii_digit())));
    let value = match hex {
        Some(digits) => u32::from_str_radix(digits, 16).ok()?,
        None => text.parse::<u32>().ok()?
    };
    u16::try_from(value).ok()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn names_and_offsets() {
        let mut table = SymbolTable::default();
        table.insert("start", 0x4000);
        table.insert("loop", 0x4010);
        assert_eq!(table.describe(0x4000).as_deref(), Some("start"));
        assert_eq!(table.describe(0x4003).as_deref(), Some("start+3"));
        assert_eq!(table.describe(0x4012).as_deref(), Some("loop+2"));
        assert_eq!(table.describe(0x3fff), None);
        assert_eq!(table.describe(0x5000), None, "too far");
        assert_eq!(table.resolve("LOOP"), Some(0x4010));
        assert_eq!(table.resolve("loop+2"), Some(0x4012));
        assert_eq!(table.resolve("start - 1"), Some(0x3fff));
        assert_eq!(table.resolve("#38"), Some(0x38));
        assert_eq!(table.resolve("100"), Some(100));
        assert_eq!(table.resolve("nowhere"), None);
        table.insert("loop", 0x4020);
        assert_eq!(table.name_at(0x4010), None, "the name moved");
        assert_eq!(table.name_at(0x4020), Some("loop"));
    }

    #[test]
    fn symbol_files_of_the_assemblers() {
        let (table, ignored) = SymbolTable::parse_file(
            "; a comment\n\
             start equ #4000\n\
             loop: EQU 0x4010\n\
             count = 12\n\
             screen #C000\n\
             data $8000 B0\n\
             9000 buffer\n\
             what is this\n"
        );
        assert_eq!(table.address_of("start"), Some(0x4000));
        assert_eq!(table.address_of("loop"), Some(0x4010));
        assert_eq!(table.address_of("count"), Some(12));
        assert_eq!(table.address_of("screen"), Some(0xc000));
        assert_eq!(table.address_of("data"), Some(0x8000));
        assert_eq!(table.address_of("buffer"), Some(0x9000));
        assert_eq!(ignored, vec!["what is this".to_owned()]);
    }

    #[test]
    fn snapshot_chunks() {
        let mut table = SymbolTable::default();
        table.add_snapshot_chunk("REMU", b"brk 4000 0;label main 16384 0;alias SCREEN 0xc000;");
        // an ACE record: length, name, map type/bank/page, 2 bytes, symbol type, address (big endian)
        let mut symb = vec![4];
        symb.extend_from_slice(b"loop");
        symb.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0x40, 0x10]);
        table.add_snapshot_chunk("SYMB", &symb);
        assert_eq!(table.address_of("main"), Some(0x4000));
        assert_eq!(table.address_of("SCREEN"), Some(0xc000));
        assert_eq!(table.address_of("loop"), Some(0x4010));
        assert_eq!(table.len(), 3);
    }
}
