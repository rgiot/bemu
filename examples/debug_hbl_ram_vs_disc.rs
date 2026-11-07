//! After HBL's in-demo loader finished reading the disc (it parks files in
//! extra RAM, to be copied back later), checks how many of the disc's
//! non-empty sectors can be found verbatim somewhere in physical RAM.
//!
//! `cargo run --release --example debug_hbl_ram_vs_disc -- <HBL.DSK>`

use amsrust::csl_runner::CslRunner;
use amsrust::disc_image::load_dsk;
use amsrust::memory::{BankNumber, PageNumber};

/// Reads an AMSDOS file's bytes (header included) straight off a data-format
/// image: directory in the first blocks, 1K blocks of 2 sectors, 9 sectors
/// (R=0xC1..0xC9) per track starting at track 0.
fn extract(image: &amsrust::disc_image::DiscImage, wanted: &str) -> Vec<u8> {
    let sector = |block: usize, half: usize| -> Vec<u8> {
        let n = block * 2 + half;
        let (track, idx) = (n / 9, n % 9);
        let t = image.track(0, track as u8).expect("track");
        t.sectors
            .iter()
            .find(|s| s.r == 0xc1 + idx as u8)
            .expect("sector")
            .data
            .clone()
    };
    let mut dir = Vec::new();
    for b in 0..2 {
        for h in 0..2 {
            dir.extend(sector(b, h));
        }
    }
    let mut extents: Vec<(u32, Vec<u8>, u8)> = Vec::new();
    for e in dir.chunks(32) {
        if e[0] == 0xe5 || e.len() < 32 {
            continue;
        }
        let name: String = e[1..9]
            .iter()
            .map(|b| (b & 0x7f) as char)
            .collect::<String>()
            .trim_end()
            .to_string()
            + "."
            + e[9..12]
                .iter()
                .map(|b| (b & 0x7f) as char)
                .collect::<String>()
                .trim_end();
        if name == wanted {
            extents.push((e[12] as u32 + 32 * e[14] as u32, e[16..32].to_vec(), e[15]));
        }
    }
    extents.sort_by_key(|e| e.0);
    let mut out = Vec::new();
    for (_, blocks, rc) in &extents {
        let mut remaining = *rc as usize * 128;
        for &b in blocks.iter().filter(|b| **b != 0) {
            for h in 0..2 {
                let d = sector(b as usize, h);
                let take = d.len().min(remaining);
                out.extend(&d[..take]);
                remaining -= take;
            }
        }
    }
    out
}

fn main() {
    let dsk = std::env::args().nth(1).expect("usage: <HBL.DSK>");
    let image = load_dsk(&std::fs::read(&dsk).unwrap()).unwrap();
    let mut runner = CslRunner::new("/tmp/debug_hbl_ram_vs_disc_shots");
    let boot = format!(
        "csl_version 1.0\ncrtc_select 0\nreset\ndisk_insert '{dsk}'\nwait 3000000\n\
         key_delay 70000 70000 400000\nkey_output 'CAT\\(RET)'\nwait 6000000\n\
         key_output 'RUN\"HBL\\(RET)'\nwait 3000000\n"
    );
    runner.run_text(&boot, ".").expect("boot");
    let mb = runner.motherboard_mut();
    for _ in 0..std::env::var("STEPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000u64)
    {
        mb.execute_next_opcode_no_video();
    }
    // physical RAM: base 64K + every extra page
    let mut ram: Vec<(String, Vec<u8>)> = Vec::new();
    let banks = [
        BankNumber::Bank0,
        BankNumber::Bank1,
        BankNumber::Bank2,
        BankNumber::Bank3,
    ];
    let mut pages = vec![("base".to_string(), PageNumber::Base)];
    for p in 0..mb.extra_ram_pages() as u8 {
        pages.push((format!("extra{p}"), PageNumber::Page(p)));
    }
    for (name, page) in pages {
        let mut bytes = Vec::new();
        for bank in banks {
            for off in 0..0x4000u16 {
                bytes.push(mb.read_physical_ram(page, bank, off));
            }
        }
        ram.push((name, bytes));
    }
    if let Some(file) = std::env::args().nth(2) {
        let full = extract(&image, &file);
        println!(
            "{file}: {} bytes on disc, header load={:02x}{:02x} exec={:02x}{:02x} len={:02x}{:02x}",
            full.len(),
            full[22],
            full[21],
            full[27],
            full[26],
            full[25],
            full[24]
        );
        let body = full[128..].to_vec();
        println!("  body[0..8]={:02x?}", &body[..8]);
        // longest verbatim prefix/any 16-byte probe positions of the body, anywhere
        for probe in [0usize, 0x100, 0x1000, 0x2000, 0x3000] {
            if probe + 16 > body.len() {
                continue;
            }
            let hits: Vec<_> = ram
                .iter()
                .flat_map(|(n, r)| {
                    r.windows(16)
                        .enumerate()
                        .filter(|(_, w)| **w == body[probe..probe + 16])
                        .map(move |(i, _)| format!("{n}:{i:#06x}"))
                        .collect::<Vec<_>>()
                })
                .collect();
            println!("  16-byte probe @{probe:#x}: {hits:?}");
        }
        let chunks: Vec<&[u8]> = body.chunks(256).filter(|c| c.len() == 256).collect();
        let hits = chunks
            .iter()
            .filter(|c| ram.iter().any(|(_, r)| r.windows(256).any(|w| w == **c)))
            .count();
        println!(
            "{file}: {hits}/{} 256-byte chunks found verbatim in physical RAM",
            chunks.len()
        );
        return;
    }
    let (mut found, mut missing, mut empty) = (0, 0, 0);
    let mut per_track: std::collections::BTreeMap<u8, (u32, u32)> = Default::default();
    for head in 0..image.nb_heads() {
        for cyl in 0..image.nb_tracks() {
            let Some(track) = image.track(head, cyl) else {
                continue;
            };
            for s in &track.sectors {
                let d = &s.data;
                if d.iter().all(|b| *b == d[0]) {
                    empty += 1;
                    continue;
                }
                // 64-byte probe from the middle, then verify the whole sector
                let mut hit = None;
                for (name, bytes) in &ram {
                    let mut start = 0;
                    while let Some(pos) = bytes[start..]
                        .windows(d.len().min(64))
                        .position(|w| w == &d[..d.len().min(64)])
                    {
                        let at = start + pos;
                        if bytes.len() >= at + d.len() && bytes[at..at + d.len()] == d[..] {
                            hit = Some((name.clone(), at));
                            break;
                        }
                        start = at + 1;
                    }
                    if hit.is_some() {
                        break;
                    }
                }
                let e = per_track.entry(cyl).or_default();
                if hit.is_some() {
                    found += 1;
                    e.0 += 1;
                } else {
                    missing += 1;
                    e.1 += 1;
                }
            }
        }
    }
    println!("sectors: {found} found in RAM, {missing} not found, {empty} uniform");
    println!("per track (found, missing): {per_track:?}");
}
