//! End-to-end test of `crate::expansion::LargeRamExpansion` through the
//! real CPU/pin-sync/memory pipeline (not just the chip's own unit tests in
//! `src/expansion.rs`, and not just `RamConfig::with_expansion_high_bits`'s
//! own unit test in `src/memory.rs`): a real Z80 `OUT` instruction, through
//! `Motherboard::tick()`, must actually land in a genuinely distinct 64kb
//! bank when a >512K expansion is plugged in.
//!
//! Protocol reference: <https://www.cpcwiki.eu/index.php/Standard_Memory_Expansions#Extended_1M-4M_Expansions_.28RAM7.2FYarek-style.29>

use amsrust::motherboard::MotherboardBuilder;

/// Runs `LD BC,{port} : LD A,{data} : OUT (C),A` at 0x8000, padded with
/// `NOP`s (`OUT (C),A` outputs register A - not C - to the port BC names,
/// which is what lets a test freely choose the port's own low byte
/// independently of the data byte, unlike `OUT (C),C`). No `HALT`: this
/// steps a generous, padded-safe number of opcodes instead of guessing the
/// exact count a raw `set_pc` needs for an `ED`-prefixed instruction to
/// fully retire, so there's no halted-CPU state and no risk of running off
/// into whatever memory follows either.
fn select_ram_config(mb: &mut amsrust::motherboard::Motherboard, port: u16, data: u8) {
    let [lo, hi] = port.to_le_bytes();
    mb.write_buffer(
        0x8000,
        &[0x01, lo, hi, 0x3e, data, 0xed, 0x79, 0x00, 0x00, 0x00],
    );
    mb.cpu_mut().set_pc(0x8000);
    for _ in 0..6 {
        mb.execute_next_opcode_no_video();
    }
}

/// Port &7Fxx must keep working exactly as it did before this expansion
/// existed (see `crate::memory`'s own `extra_pages_are_independent_when_actually_fitted`
/// for the pre-expansion, `set_extra_ram_pages`-only equivalent of this) -
/// the expansion's high bits are 0 there, a no-op combine.
#[test]
fn port_7f_still_behaves_like_plain_512k_hardware() {
    let mut mb = MotherboardBuilder::default()
        .plug_large_ram_expansion(64)
        .disable_upper_rom()
        .build();

    for bank in 0u8..8 {
        // config 1 (`ccc=001`): substitutes 0xC000-0xFFFF with page `bbb`.
        let data = 0b1100_0000 | (bank << 3) | 0b001;
        select_ram_config(&mut mb, 0x7fff, data);
        mb.write_byte(0xC000, 0x10 + bank);
    }
    for bank in 0u8..8 {
        let data = 0b1100_0000 | (bank << 3) | 0b001;
        select_ram_config(&mut mb, 0x7fff, data);
        assert_eq!(
            mb.read_byte(0xC000),
            0x10 + bank,
            "bank {bank} via port 0x7Fxx"
        );
    }
}

/// The actual point of `LargeRamExpansion`: successive ports (0x7F, 0x7E,
/// ..., 0x78) reach *different* 64K banks for the *same* data byte, giving
/// real access to more than the standard 512K - confirmed here by writing
/// a distinct value through each of the 8 ports and reading every one back
/// independently, exactly the kind of check `AmstradDiag.dsk`'s own boot
/// self-test performs (see `tests/amstrad_diagnostics.rs` and
/// `project_amstrad_diagnostics_tool` memory for why that tool doesn't yet
/// confirm this specific case itself).
#[test]
fn successive_ports_reach_genuinely_independent_banks_beyond_512k() {
    let mut mb = MotherboardBuilder::default()
        .plug_large_ram_expansion(64)
        .disable_upper_rom()
        .build();
    assert_eq!(mb.large_ram_expansion_bank_count(), Some(64));
    // The CPC's own on-board extra page is untouched by plugging a card in
    // (see `Motherboard::plug_large_ram_expansion`'s doc comment) - it just
    // becomes unreachable while one's plugged in, not resized away.
    assert_eq!(mb.extra_ram_pages(), 1);

    let ports = [
        0x7fffu16, 0x7eff, 0x7dff, 0x7cff, 0x7bff, 0x7aff, 0x79ff, 0x78ff,
    ];
    let config = 0b1100_1001u8; // ccc=001, bbb=001: substitutes 0xC000-0xFFFF with the selected bank

    for (i, &port) in ports.iter().enumerate() {
        select_ram_config(&mut mb, port, config);
        mb.write_byte(0xC000, 0x40 + i as u8);
    }
    for (i, &port) in ports.iter().enumerate() {
        select_ram_config(&mut mb, port, config);
        assert_eq!(
            mb.read_byte(0xC000),
            0x40 + i as u8,
            "port {port:04x} (the {i}th 512K block) should hold its own independent value"
        );
    }
}

/// Unplugging reverts to the stock machine's own single extra page -
/// selecting a port beyond 0x7Fxx must alias back exactly like an
/// unexpanded machine does (see `src/memory.rs`'s
/// `a_page_beyond_what_is_fitted_aliases_instead_of_panicking`), not panic
/// or keep reaching whatever the expansion used to provide.
#[test]
fn unplugging_reverts_to_stock_aliasing_behaviour() {
    let mut mb = MotherboardBuilder::default()
        .plug_large_ram_expansion(64)
        .disable_upper_rom()
        .build();
    select_ram_config(&mut mb, 0x78ff, 0b1100_1001);
    mb.write_byte(0xC000, 0xaa);

    mb.unplug_large_ram_expansion();
    assert_eq!(mb.extra_ram_pages(), 1);

    // Port 0x78 no longer means anything special post-unplug: any port in
    // the standard decode range behaves like plain 0x7Fxx, aliasing to the
    // one fitted page - writing through 0x7Fxx and reading back confirms
    // it's the *same* page 0x78xx would also reach now.
    select_ram_config(&mut mb, 0x7fff, 0b1100_1001);
    mb.write_byte(0xC000, 0x55);
    select_ram_config(&mut mb, 0x78ff, 0b1100_1001);
    assert_eq!(
        mb.read_byte(0xC000),
        0x55,
        "post-unplug, every port should alias to the one fitted page"
    );
}
