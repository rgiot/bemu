//! The Z80 window: its registers (those changed lately lit) and its state.

use super::*;

/// The Z80's state beyond its registers: HALT, WZ, the bytes the pointers point at.
pub fn z80(ui: &mut egui::Ui, mb: &Motherboard, state: &crate::state::Z80Registers) {
    use crate::z80::{HasValue, Reg16};
    let cpu = mb.cpu();
    let r16 = |reg: Reg16| cpu.reg16(reg).value();
    egui::Grid::new("z80-more").striped(true).show(ui, |ui| {
        ui.label("State");
        ui.monospace(if state.halted {
            "HALT, waiting for an interrupt"
        } else {
            "running"
        });
        ui.end_row();
        ui.label("WZ (MEMPTR)");
        ui.monospace(format!("&{:04X}", state.WZ));
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
/// The registers of the Z80 window, in their [`RegisterHeat`]'s order (then I and R).
const HEAT_REGISTERS: [crate::z80::Reg16; 12] = {
    use crate::z80::Reg16::*;
    [Af, Bc, De, Hl, Af2, Bc2, De2, Hl2, Ix, Iy, Sp, Pc]
};

/// The registers changed lately (see [`crate::ui::heat`]).
pub type RegisterHeat = crate::ui::heat::Heat;

pub fn registers(
    ui: &mut egui::Ui,
    mb: &Motherboard,
    bus: &mut CommandBus,
    heat: &mut RegisterHeat,
) {
    use crate::z80::{HasValue, Reg16, Reg8};
    {
        use crate::z80::HasValue as _;
        let cpu = mb.cpu();
        let mut values = [0u32; 14];
        for (value, reg) in values.iter_mut().zip(HEAT_REGISTERS) {
            *value = u32::from(cpu.reg16(reg).value());
        }
        values[12] = u32::from(cpu.reg8(Reg8::I).value());
        values[13] = u32::from(cpu.reg8(Reg8::R).value());
        crate::ui::heat::observe(ui, heat, 0, mb, &values);
    }
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
        let response = crate::ui::heat::field(ui, heat.heat(index(reg)), |ui| {
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
                if crate::ui::heat::field(ui, heat.heat(slot), |ui| {
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
