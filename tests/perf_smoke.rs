//! Fast, self-contained throughput benchmark for `Motherboard::tick()` -
//! the hot path `project_cpu_tick_performance` profiled (`cpu.tick()` ~52%,
//! `sync_chips_pins()` ~20% of tick time). zexdoc/zexall are *useless* for
//! measuring a perf change in practice (minutes per run just to move the
//! needle a little, hours to finish) - this runs in a couple of seconds by
//! driving `tick()` directly, with memory filled with NOP so almost all
//! measured time is per-tick/pin-sync overhead rather than any particular
//! instruction's own semantic work (the thing being optimized, not the
//! thing incidental to it).
//!
//! `cargo test --release --test perf_smoke -- --ignored --nocapture`
//! (ignored since it's a benchmark, not a correctness check - nothing here
//! asserts on timing, only prints it, so it's never flaky in CI).

use amsrust::motherboard::MotherboardBuilder;
use std::time::Instant;

#[test]
#[ignore = "benchmark, not a correctness check - run manually to compare before/after a perf change"]
fn tick_throughput() {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();

    // NOP everywhere: PC just runs off the end and wraps, forever, with
    // (almost) zero per-instruction semantic cost - isolates per-tick
    // overhead as cleanly as a whole-machine (not just the Z80 core in
    // isolation) benchmark can.
    let nops = vec![0u8; 0x10000];
    motherboard.write_buffer(0, &nops);

    const WARMUP_TICKS: u64 = 1_000_000;
    const MEASURED_TICKS: u64 = 50_000_000;

    for _ in 0..WARMUP_TICKS {
        motherboard.tick();
    }

    let start = Instant::now();
    for _ in 0..MEASURED_TICKS {
        motherboard.tick();
    }
    let elapsed = start.elapsed();

    let ticks_per_sec = MEASURED_TICKS as f64 / elapsed.as_secs_f64();
    eprintln!(
        "[perf_smoke] {MEASURED_TICKS} ticks in {elapsed:.3?} ({ticks_per_sec:.0} ticks/s, {:.1} ns/tick)",
        elapsed.as_nanos() as f64 / MEASURED_TICKS as f64
    );
}
