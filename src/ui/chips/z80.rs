//! The Z80 window: its registers (those changed lately lit) and its state.

use super::*;

/// The Z80's state beyond its registers: HALT, WZ, the bytes the pointers point at.
pub fn z80(ui: &mut egui::Ui, mb: &Motherboard) {
    use crate::z80::{HasValue, Reg16};
    let cpu = mb.cpu();
    let r16 = |reg: Reg16| cpu.reg16(reg).value();
    let pc = r16(Reg16::Pc);
    egui::Grid::new("z80-more").striped(true).show(ui, |ui| {
        ui.label("State");
        // a HALT runs again until an interrupt: the PC stays on it
        ui.monospace(if mb.read_byte(pc) == 0x76 {
            "HALT, waiting for an interrupt"
        } else {
            "running"
        });
        ui.end_row();
        ui.label("WZ (MEMPTR)");
        ui.monospace(format!("&{:04X}", r16(Reg16::Wz)));
        ui.end_row();
        for (name, reg) in [
            ("(HL)", Reg16::Hl),
            ("(DE)", Reg16::De),
            ("(BC)", Reg16::Bc),
            ("(SP)", Reg16::Sp),
            ("(IX)", Reg16::Ix),
            ("(IY)", Reg16::Iy),
        ] {
            let at = r16(reg);
            let bytes: Vec<String> = (0..8u16)
                .map(|i| format!("{:02X}", mb.read_byte(at.wrapping_add(i))))
                .collect();
            ui.label(name);
            ui.monospace(format!("&{at:04X}: {}", bytes.join(" ")));
            ui.end_row();
        }
    });
}

/// The Z80's registers, editable (each change a `z80-set-register` command): the flags first,
/// each a check box; then each pair beside its shadow (AF / AF'...), the index registers, SP and
/// PC, I and R.
/// The registers of the Z80 window, in [`RegisterHeat`]'s order.
const HEAT_REGISTERS: [crate::z80::Reg16; 12] = {
    use crate::z80::Reg16::*;
    [Af, Bc, De, Hl, Af2, Bc2, De2, Hl2, Ix, Iy, Sp, Pc]
};

/// The steps over which a register changed fades out.
const HEAT_STEPS: u64 = 4;

/// Which registers the machine changed lately: each time it moved (a step, a frame), the
/// registers it changed are lit, and fade over the next [`HEAT_STEPS`] moves. A paused machine
/// keeps its colours (what its last step changed).
#[derive(Default)]
pub struct RegisterHeat {
    /// The 16-bit registers, then I and R.
    last: Option<[u16; 14]>,
    /// The moves seen.
    step: u64,
    /// The move that last changed each register (0: none seen).
    changed: [u64; 14],
}

impl RegisterHeat {
    fn observe(&mut self, mb: &Motherboard) {
        use crate::z80::{HasValue, Reg8};
        let cpu = mb.cpu();
        let mut values = [0u16; 14];
        for (value, reg) in values.iter_mut().zip(HEAT_REGISTERS) {
            *value = cpu.reg16(reg).value();
        }
        values[12] = u16::from(cpu.reg8(Reg8::I).value());
        values[13] = u16::from(cpu.reg8(Reg8::R).value());
        self.observe_values(values);
    }

    fn observe_values(&mut self, values: [u16; 14]) {
        if self.last == Some(values) {
            return;
        }
        self.step += 1;
        if let Some(last) = self.last {
            for (i, (old, new)) in last.iter().zip(values).enumerate() {
                if *old != new {
                    self.changed[i] = self.step;
                }
            }
        }
        self.last = Some(values);
    }

    /// 1.0: changed by the last move, less for older ones, 0.0 none lately.
    fn heat(&self, index: usize) -> f32 {
        let changed = self.changed[index];
        let age = self.step - changed;
        if changed == 0 || age >= HEAT_STEPS {
            0.0
        } else {
            1.0 - age as f32 / HEAT_STEPS as f32
        }
    }
}

/// A value field lit by its heat (see [`RegisterHeat`]), as the PSG window's registers.
fn heated<R>(ui: &mut egui::Ui, heat: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let fill = if heat > 0.0 {
        egui::Color32::from_rgb(200, 140, 30).gamma_multiply(0.25 + 0.6 * heat)
    } else {
        egui::Color32::TRANSPARENT
    };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(3.0)
        .show(ui, add)
        .inner
}

pub fn registers(
    ui: &mut egui::Ui,
    mb: &Motherboard,
    bus: &mut CommandBus,
    heat: &mut RegisterHeat,
) {
    use crate::z80::{HasValue, Reg16, Reg8};
    heat.observe(mb);
    let heat = &*heat;
    let index = |reg: Reg16| HEAT_REGISTERS.iter().position(|r| *r == reg).unwrap_or(0);
    let cpu = mb.cpu();
    // the flags, each a check box: F set again with that bit changed
    let af = cpu.reg16(Reg16::Af).value();
    let f = af as u8;
    ui.horizontal(|ui| {
        ui.strong("Flags");
        for (bit, name) in ["S", "Z", "Y", "H", "X", "P/V", "N", "C"]
            .iter()
            .enumerate()
        {
            let mask = 0x80u8 >> bit;
            let mut on = f & mask != 0;
            if ui.checkbox(&mut on, *name).changed() {
                let flags = if on { f | mask } else { f & !mask };
                run(
                    bus,
                    "z80-set-register",
                    Arg::Text(format!("AF={}", (af & 0xff00) | u16::from(flags))),
                );
            }
        }
    });
    ui.add_space(4.0);
    // a 16-bit register to edit: its name, its field, its value in decimal
    let word = |ui: &mut egui::Ui, bus: &mut CommandBus, name: &str, reg: Reg16| {
        let value = cpu.reg16(reg).value();
        ui.monospace(name);
        let mut edited = u32::from(value);
        let response = heated(ui, heat.heat(index(reg)), |ui| {
            ui.add(
                egui::DragValue::new(&mut edited)
                    .range(0..=0xffff)
                    .hexadecimal(4, false, true)
                    .prefix("&")
                    .speed(1.0),
            )
        });
        if response.changed() && edited != u32::from(value) {
            run(
                bus,
                "z80-set-register",
                Arg::Text(format!("{name}={edited}")),
            );
        }
        ui.weak(format!("{value:5}"));
    };
    egui::Grid::new("z80-registers")
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            // each pair on the row of its shadow
            for (name, reg, shadow, shadow_reg) in [
                ("AF", Reg16::Af, "AF'", Reg16::Af2),
                ("BC", Reg16::Bc, "BC'", Reg16::Bc2),
                ("DE", Reg16::De, "DE'", Reg16::De2),
                ("HL", Reg16::Hl, "HL'", Reg16::Hl2),
            ] {
                word(ui, bus, name, reg);
                word(ui, bus, shadow, shadow_reg);
                ui.end_row();
            }
            word(ui, bus, "IX", Reg16::Ix);
            word(ui, bus, "IY", Reg16::Iy);
            ui.end_row();
            word(ui, bus, "SP", Reg16::Sp);
            word(ui, bus, "PC", Reg16::Pc);
            ui.end_row();
            for (name, reg, slot) in [("I", Reg8::I, 12), ("R", Reg8::R, 13)] {
                let value = cpu.reg8(reg).value();
                ui.monospace(name);
                let mut edited = u32::from(value);
                if heated(ui, heat.heat(slot), |ui| {
                    ui.add(
                        egui::DragValue::new(&mut edited)
                            .range(0..=0xff)
                            .hexadecimal(2, false, true)
                            .prefix("&"),
                    )
                })
                .changed()
                    && edited != u32::from(value)
                {
                    run(
                        bus,
                        "z80-set-register",
                        Arg::Text(format!("{name}={edited}")),
                    );
                }
                ui.weak(format!("{value:5}"));
            }
            ui.end_row();
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A register changed by the last move is the hottest; it cools over the next moves, and a
    /// machine that does not move keeps its colours.
    #[test]
    fn the_registers_changed_lately_are_lit() {
        let mut heat = RegisterHeat::default();
        let mut values = [0u16; 14];
        heat.observe_values(values);
        assert_eq!(heat.heat(0), 0.0, "nothing changed yet");
        values[1] = 0x1234; // BC
        heat.observe_values(values);
        assert_eq!(heat.heat(1), 1.0);
        assert_eq!(heat.heat(0), 0.0);
        // paused: the same values, the same colours
        heat.observe_values(values);
        assert_eq!(heat.heat(1), 1.0);
        values[11] = 1; // PC
        heat.observe_values(values);
        assert!(heat.heat(1) < 1.0 && heat.heat(1) > 0.0);
        assert_eq!(heat.heat(11), 1.0);
        for pc in 2..2 + HEAT_STEPS as u16 {
            values[11] = pc;
            heat.observe_values(values);
        }
        assert_eq!(heat.heat(1), 0.0, "cooled down");
    }
}
