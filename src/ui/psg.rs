//! Debug > View > PSG: the AY-3-8912 as a musician reads it - the 16 registers (lit when they
//! change), per channel its period, note (and how far from it), mixer and volume, the notes on a
//! keyboard, the envelope's shape drawn, and the channels' levels of the last frames.

use std::collections::VecDeque;

use crate::motherboard::Motherboard;

/// The AY's clock on a CPC: 1 MHz.
const PSG_HZ: f64 = 1_000_000.0;
/// The channels' colours, on the keyboard and the levels.
const COLOURS: [egui::Color32; 3] = [
    egui::Color32::from_rgb(230, 80, 80),
    egui::Color32::from_rgb(80, 200, 100),
    egui::Color32::from_rgb(90, 140, 240),
];
const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// How many frames of levels the strip keeps.
const HISTORY: usize = 150;
/// How long a changed register stays lit.
const LIT_S: f64 = 1.0;

/// What the window remembers from one drawing to the next.
#[derive(Clone, Default)]
struct Memory {
    registers: [u8; 16],
    /// When each register last changed (egui's time).
    changed: [f64; 16],
    levels: VecDeque<[f32; 3]>,
    /// The machine's time of the last levels kept: a paused machine adds none.
    t_state: u64,
}

/// A tone's period (R0-R5) as a frequency.
pub fn tone_hz(period: u16) -> f64 {
    PSG_HZ / (16.0 * f64::from(period.max(1)))
}

/// The nearest note (MIDI number) and how far from it, in cents; `None` out of hearing.
pub fn nearest_note(frequency: f64) -> Option<(i32, i32)> {
    if !(16.0..20_000.0).contains(&frequency) {
        return None;
    }
    let exact = 69.0 + 12.0 * (frequency / 440.0).log2();
    let midi = exact.round();
    Some((midi as i32, ((exact - midi) * 100.0).round() as i32))
}

fn note_name(midi: i32) -> String {
    format!("{}{}", NAMES[midi.rem_euclid(12) as usize], midi / 12 - 1)
}

/// The envelope's level (0-15) along 3 periods of `shape` (R13), 32 steps a period, as the AY
/// makes it: shapes 0-7 go once then stay at 0; 8-15 repeat, hold or alternate.
pub fn envelope_levels(shape: u8) -> Vec<u8> {
    let attack = shape & 4 != 0;
    let (cont, alternate, hold) = if shape & 8 == 0 {
        (false, false, true)
    } else {
        (true, shape & 2 != 0, shape & 1 != 0)
    };
    let mut levels = Vec::with_capacity(96);
    let mut up = attack;
    for period in 0..3 {
        for step in 0..32u8 {
            let level = step / 2;
            levels.push(if period == 0 || (cont && !hold) {
                if up {
                    level
                } else {
                    15 - level
                }
            } else if !cont {
                0
            } else {
                // held: the end of the first period, or its opposite when alternating
                let end_high = attack != alternate;
                if end_high {
                    15
                } else {
                    0
                }
            });
        }
        if cont && !hold && alternate {
            up = !up;
        }
    }
    levels
}

pub fn show(ui: &mut egui::Ui, mb: &Motherboard) {
    let psg = mb.psg();
    let reg = |r: u8| psg.register(r);
    let now = ui.input(|i| i.time);
    let id = egui::Id::new("psg-window");
    let mut memory: Memory = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    for r in 0..16u8 {
        if memory.registers[usize::from(r)] != reg(r) {
            memory.registers[usize::from(r)] = reg(r);
            memory.changed[usize::from(r)] = now;
        }
    }
    let t_state = mb.t_state_counter().value() as u64;
    if t_state != memory.t_state {
        memory.t_state = t_state;
        memory.levels.push_back(psg.channel_levels());
        while memory.levels.len() > HISTORY {
            memory.levels.pop_front();
        }
    }

    registers(ui, mb, &memory, now);
    ui.add_space(4.0);
    let mixer = reg(7);
    let mut notes = [None; 3];
    egui::Grid::new("psg-channels")
        .striped(true)
        .show(ui, |ui| {
            for heading in ["", "Period", "Frequency", "Note", "Tone", "Noise", "Volume"] {
                ui.strong(heading);
            }
            ui.end_row();
            for channel in 0..3u8 {
                let period =
                    u16::from(reg(channel * 2)) | u16::from(reg(channel * 2 + 1) & 0x0f) << 8;
                let frequency = tone_hz(period);
                let volume = reg(8 + channel);
                let tone = mixer & (1 << channel) == 0;
                ui.colored_label(
                    COLOURS[usize::from(channel)],
                    egui::RichText::new(["A", "B", "C"][usize::from(channel)]).strong(),
                );
                ui.monospace(format!("{period} (&{period:03X})"));
                ui.monospace(format!("{frequency:.1} Hz"));
                match nearest_note(frequency) {
                    Some((midi, cents)) => {
                        ui.monospace(format!("{} {cents:+}¢", note_name(midi)));
                        if tone && volume != 0 {
                            notes[usize::from(channel)] = Some(midi);
                        }
                    }
                    None => {
                        ui.monospace("-");
                    }
                }
                ui.monospace(if tone { "on" } else { "off" });
                ui.monospace(if mixer & (8 << channel) == 0 {
                    "on"
                } else {
                    "off"
                });
                ui.monospace(if volume & 0x10 != 0 {
                    "envelope".to_owned()
                } else {
                    format!("{}", volume & 0x0f)
                });
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    keyboard(ui, &notes);
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            let noise = reg(6) & 0x1f;
            let envelope = u16::from(reg(11)) | u16::from(reg(12)) << 8;
            let shape = reg(13) & 0x0f;
            ui.monospace(format!(
                "Noise     period {noise:2}: {:.0} Hz",
                PSG_HZ / (16.0 * f64::from(noise.max(1)))
            ));
            ui.monospace(format!(
                "Envelope  period {envelope}: {:.2} Hz ({:.1} ms), shape {shape}",
                PSG_HZ / (256.0 * f64::from(envelope.max(1))),
                256.0 * f64::from(envelope.max(1)) / 1000.0
            ));
            envelope_graph(ui, shape);
            ui.monospace(format!(
                "Port A    &{:02X} ({})",
                reg(14),
                if psg.port_a_is_input() {
                    "input: the keyboard"
                } else {
                    "output"
                }
            ));
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.weak("Levels, the last 3 seconds");
            level_strip(ui, &memory.levels);
        });
    });
    ui.data_mut(|d| d.insert_temp(id, memory));
}

/// R0-R15 in boxes, lit for a second after a change, the selected one outlined.
fn registers(ui: &mut egui::Ui, mb: &Motherboard, memory: &Memory, now: f64) {
    const ROLES: [&str; 16] = [
        "A fine",
        "A coarse",
        "B fine",
        "B coarse",
        "C fine",
        "C coarse",
        "Noise",
        "Mixer",
        "A volume",
        "B volume",
        "C volume",
        "Env fine",
        "Env coarse",
        "Env shape",
        "Port A",
        "Port B",
    ];
    let selected = mb.psg().selected_register();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
        for r in 0..16u8 {
            let value = memory.registers[usize::from(r)];
            let age = now - memory.changed[usize::from(r)];
            let lit = memory.changed[usize::from(r)] > 0.0 && age < LIT_S;
            let visuals = ui.visuals().clone();
            let fill = if lit {
                // fades out
                let strength = (1.0 - age / LIT_S) as f32;
                egui::Color32::from_rgb(200, 140, 30).gamma_multiply(0.3 + 0.7 * strength)
            } else {
                visuals.extreme_bg_color
            };
            let stroke = if selected == Some(r) {
                egui::Stroke::new(2.0_f32, visuals.selection.stroke.color)
            } else {
                egui::Stroke::new(1.0_f32, visuals.weak_text_color())
            };
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(42.0, 30.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 3.0, fill);
            painter.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);
            painter.text(
                rect.center_top() + egui::vec2(0.0, 2.0),
                egui::Align2::CENTER_TOP,
                format!("R{r}"),
                egui::FontId::proportional(9.0),
                visuals.weak_text_color(),
            );
            painter.text(
                rect.center_bottom() - egui::vec2(0.0, 3.0),
                egui::Align2::CENTER_BOTTOM,
                format!("&{value:02X}"),
                egui::FontId::monospace(12.0),
                if lit {
                    egui::Color32::BLACK
                } else {
                    visuals.text_color()
                },
            );
            response.on_hover_text(format!(
                "R{r}: {} = {value} ({value:#010b})",
                ROLES[usize::from(r)]
            ));
        }
        if lit_any(memory, now) {
            ui.ctx().request_repaint();
        }
    });
}

fn lit_any(memory: &Memory, now: f64) -> bool {
    memory
        .changed
        .iter()
        .any(|&changed| changed > 0.0 && now - changed < LIT_S)
}

/// A piano keyboard from C1 to B8: each channel's note marked in its colour.
fn keyboard(ui: &mut egui::Ui, notes: &[Option<i32>; 3]) {
    const WHITE_W: f32 = 9.0;
    const HEIGHT: f32 = 40.0;
    const FIRST: i32 = 24; // C1
    const OCTAVES: i32 = 8;
    let whites = OCTAVES * 7;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(whites as f32 * WHITE_W + 1.0, HEIGHT + 12.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let is_black = |midi: i32| matches!(midi.rem_euclid(12), 1 | 3 | 6 | 8 | 10);
    // the white key a note is on (or right of, for a black one)
    let white_index = |midi: i32| {
        let octave = (midi - FIRST).div_euclid(12);
        let in_octave = [0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 5, 6][midi.rem_euclid(12) as usize];
        octave * 7 + in_octave
    };
    let key_rect = |midi: i32| {
        let x = rect.left() + white_index(midi) as f32 * WHITE_W;
        if is_black(midi) {
            egui::Rect::from_min_size(
                egui::pos2(x + WHITE_W * 0.65, rect.top()),
                egui::vec2(WHITE_W * 0.7, HEIGHT * 0.6),
            )
        } else {
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(WHITE_W, HEIGHT))
        }
    };
    let last = FIRST + OCTAVES * 12;
    let colour_of = |midi: i32| {
        notes
            .iter()
            .position(|&note| note == Some(midi))
            .map(|channel| COLOURS[channel])
    };
    for midi in (FIRST..last).filter(|&m| !is_black(m)) {
        let key = key_rect(midi);
        painter.rect_filled(
            key,
            1.0,
            colour_of(midi).unwrap_or(egui::Color32::from_gray(230)),
        );
        painter.rect_stroke(
            key,
            1.0,
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(60)),
            egui::StrokeKind::Inside,
        );
        if midi.rem_euclid(12) == 0 {
            painter.text(
                egui::pos2(key.left() + 1.0, rect.top() + HEIGHT + 1.0),
                egui::Align2::LEFT_TOP,
                note_name(midi),
                egui::FontId::proportional(9.0),
                ui.visuals().weak_text_color(),
            );
        }
    }
    for midi in (FIRST..last).filter(|&m| is_black(m)) {
        let key = key_rect(midi);
        painter.rect_filled(
            key,
            1.0,
            colour_of(midi).unwrap_or(egui::Color32::from_gray(25)),
        );
    }
    // a note out of the keyboard: an arrow at its edge
    for (channel, note) in notes.iter().enumerate() {
        if let Some(midi) = *note {
            if midi < FIRST || midi >= last {
                let x = if midi < FIRST {
                    rect.left() + 2.0
                } else {
                    rect.right() - 8.0
                };
                painter.text(
                    egui::pos2(x, rect.top() + HEIGHT / 2.0),
                    egui::Align2::LEFT_CENTER,
                    if midi < FIRST { "<" } else { ">" },
                    egui::FontId::monospace(12.0),
                    COLOURS[channel],
                );
            }
        }
    }
    if let Some(pointer) = response.hover_pos() {
        let hovered = (FIRST..last)
            .filter(|&m| is_black(m))
            .chain((FIRST..last).filter(|&m| !is_black(m)))
            .find(|&m| key_rect(m).contains(pointer));
        if let Some(midi) = hovered {
            let hz = 440.0 * 2f64.powf(f64::from(midi - 69) / 12.0);
            let period = (PSG_HZ / (16.0 * hz)).round();
            response.on_hover_text(format!("{}: {hz:.1} Hz, period {period}", note_name(midi)));
        }
    }
}

/// The envelope's shape over 3 periods.
fn envelope_graph(ui: &mut egui::Ui, shape: u8) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(192.0, 34.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let levels = envelope_levels(shape);
    let step = rect.width() / levels.len() as f32;
    let y = |level: u8| rect.bottom() - 2.0 - f32::from(level) / 15.0 * (rect.height() - 4.0);
    let points: Vec<egui::Pos2> = levels
        .iter()
        .enumerate()
        .flat_map(|(i, &level)| {
            let x = rect.left() + i as f32 * step;
            [egui::pos2(x, y(level)), egui::pos2(x + step, y(level))]
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(200, 140, 30)),
    ));
    // the periods' limits
    for period in 1..3 {
        let x = rect.left() + rect.width() * period as f32 / 3.0;
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color().gamma_multiply(0.4)),
        );
    }
}

/// The 3 channels' levels of the last frames, one line each, the newest on the right.
fn level_strip(ui: &mut egui::Ui, levels: &VecDeque<[f32; 3]>) {
    const LANE: f32 = 26.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(HISTORY as f32 * 2.0, LANE * 3.0 + 4.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let start = rect.right() - levels.len() as f32 * 2.0;
    for (i, frame) in levels.iter().enumerate() {
        let x = start + i as f32 * 2.0;
        for (channel, level) in frame.iter().enumerate() {
            let bottom = rect.top() + 2.0 + LANE * (channel as f32 + 1.0);
            let height = level.clamp(0.0, 1.0) * (LANE - 2.0);
            if height > 0.0 {
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x, bottom - height),
                        egui::pos2(x + 2.0, bottom),
                    ),
                    0.0,
                    COLOURS[channel],
                );
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_period_is_a_note() {
        // the firmware's A4 (440 Hz): period 142
        let (midi, cents) = nearest_note(tone_hz(142)).unwrap();
        assert_eq!(note_name(midi), "A4");
        assert!(cents.abs() < 5, "{cents}");
        assert_eq!(nearest_note(tone_hz(1)), None);
    }

    #[test]
    fn the_envelope_shapes_are_the_ays() {
        let at = |shape: u8, i: usize| envelope_levels(shape)[i];
        // \___ : down once, then 0
        assert_eq!((at(0, 0), at(0, 31), at(0, 40)), (15, 0, 0));
        // /___ : up once, then 0
        assert_eq!((at(4, 0), at(4, 31), at(4, 40)), (0, 15, 0));
        // \\\\ : down, again
        assert_eq!((at(8, 0), at(8, 31), at(8, 32)), (15, 0, 15));
        // \/\/ : down then up
        assert_eq!((at(10, 31), at(10, 63)), (0, 15));
        // \¯¯¯ : down, then held high
        assert_eq!((at(11, 31), at(11, 40)), (0, 15));
        // /¯¯¯ : up, then held high
        assert_eq!((at(13, 31), at(13, 40)), (15, 15));
        // /\/\ : up then down
        assert_eq!((at(14, 31), at(14, 63)), (15, 0));
        // /___ (15): up, then held low
        assert_eq!((at(15, 31), at(15, 40)), (15, 0));
        // \___ (9): down, held low
        assert_eq!(at(9, 40), 0);
        // //// (12)
        assert_eq!((at(12, 31), at(12, 32)), (15, 0));
    }
}
