//! What changed lately, lit the same way in every window (registers, memory bytes, the PSG,
//! the CRTC, the inks, the stack): a value is lit when it changed within the machine's last
//! [`MOVES`] moves (a step, a frame: the machine's clock went on) or the last [`LIT_S`] seconds,
//! fading out. A paused machine keeps its colours: what its last step changed stays lit.

use std::hash::Hash;

use crate::motherboard::Motherboard;

/// The moves of the machine over which a change fades out.
pub const MOVES: u64 = 4;

/// The seconds over which a change fades out (what a running machine changes, seen).
pub const LIT_S: f64 = 1.0;

/// The colour of a change, its strength `heat` (1: just changed).
pub fn fill(heat: f32) -> egui::Color32 {
    egui::Color32::from_rgb(200, 140, 30).gamma_multiply(0.25 + 0.6 * heat.clamp(0.0, 1.0))
}

/// Values watched, and when each last changed.
#[derive(Clone, Default)]
pub struct Heat {
    /// What the values are (an address, a paging): another key, other values.
    key: u64,
    last: Vec<u32>,
    /// The machine's clock at the last look.
    clock: u64,
    /// The moves of the machine seen.
    moves: u64,
    now: f64,
    /// Each value's last change: the move and the time (`None`: not seen changing).
    changed: Vec<Option<(u64, f64)>>,
}

impl Heat {
    /// Look at `values` (what `key` names) at the machine's `clock` and the time `now`.
    pub fn observe(&mut self, key: u64, clock: u64, now: f64, values: &[u32]) {
        self.now = now;
        if clock != self.clock {
            self.clock = clock;
            self.moves += 1;
        }
        if key != self.key || values.len() != self.last.len() {
            // other values: none changed
            self.key = key;
            self.last = values.to_vec();
            self.changed = vec![None; values.len()];
            return;
        }
        for (i, (old, new)) in self.last.iter_mut().zip(values).enumerate() {
            if old != new {
                *old = *new;
                self.changed[i] = Some((self.moves, now));
            }
        }
    }

    /// Value `index`'s heat by the machine's moves and by the time since its change.
    fn parts(&self, index: usize) -> Option<(f32, f32)> {
        let (moves, time) = (*self.changed.get(index)?)?;
        let by_moves = 1.0 - (self.moves - moves) as f32 / MOVES as f32;
        let by_time = (1.0 - (self.now - time) / LIT_S) as f32;
        Some((by_moves, by_time))
    }

    /// Value `index`: 1.0 just changed, less for an older change, 0.0 none lately.
    pub fn heat(&self, index: usize) -> f32 {
        self.parts(index)
            .map_or(0.0, |(by_moves, by_time)| by_moves.max(by_time).max(0.0))
    }

    /// The time still dims a colour (the window is to be drawn again to show it): not a
    /// colour the machine's moves keep, as a paused machine's last step.
    pub fn fading(&self) -> bool {
        (0..self.changed.len())
            .filter_map(|i| self.parts(i))
            .any(|(by_moves, by_time)| by_time > 0.0 && by_time > by_moves)
    }
}

/// A [`Heat`] for each of a few keys: values that come and go (a memory window following the
/// paging, the firmware switching its ROM in and out many times a frame) each compared with
/// what the same key showed before, not with the others.
#[derive(Clone, Default)]
pub struct Heats {
    /// The most recently used last.
    heats: Vec<(u64, Heat)>,
}

/// The keys a [`Heats`] keeps.
const KEYS: usize = 8;

impl Heats {
    /// The heat of `key`'s values, looked at now.
    pub fn observe(&mut self, ui: &egui::Ui, key: u64, mb: &Motherboard, values: &[u32]) -> &Heat {
        let heat = self.of(key);
        observe(ui, heat, key, mb, values);
        heat
    }

    /// The heat of `key`, made the most recently used.
    fn of(&mut self, key: u64) -> &mut Heat {
        let heat = match self.heats.iter().position(|(k, _)| *k == key) {
            Some(index) => self.heats.remove(index).1,
            None => Heat::default(),
        };
        self.heats.push((key, heat));
        if self.heats.len() > KEYS {
            self.heats.remove(0);
        }
        &mut self.heats.last_mut().expect("just pushed").1
    }
}

/// The heat of `values` (what `key` names) in the window of `ui`, kept in its context under
/// `id`: for the windows that keep no state of their own.
pub fn watch(ui: &egui::Ui, id: impl Hash + std::fmt::Debug, key: u64, mb: &Motherboard, values: &[u32]) -> Heat {
    let id = egui::Id::new(("heat", id));
    let mut heat: Heat = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    observe(ui, &mut heat, key, mb, values);
    ui.data_mut(|d| d.insert_temp(id, heat.clone()));
    heat
}

/// [`Heat::observe`] with the machine's clock and the window's time; a change fading out asks
/// for the next frame.
pub fn observe(ui: &egui::Ui, heat: &mut Heat, key: u64, mb: &Motherboard, values: &[u32]) {
    let now = ui.input(|i| i.time);
    heat.observe(key, mb.t_state_counter().value() as u64, now, values);
    if heat.fading() {
        ui.ctx().request_repaint();
    }
}

/// A field (a value to edit, a button) lit by its heat: its own background (a frame's fill
/// would be under it, hidden).
pub fn field<R>(ui: &mut egui::Ui, heat: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if heat <= 0.0 {
        return add(ui);
    }
    let fill = fill(heat);
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.weak_bg_fill = fill;
        widgets.inactive.bg_fill = fill;
        add(ui)
    })
    .inner
}

/// A text lit by its heat.
pub fn text(text: egui::RichText, heat: f32) -> egui::RichText {
    if heat > 0.0 {
        text.background_color(fill(heat))
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change is the hottest at the move it happened; it cools over the next moves (the time
    /// here passing too slowly to keep it), and a machine that does not move keeps it.
    #[test]
    fn a_change_fades_over_the_machine_moves() {
        let mut heat = Heat::default();
        let mut values = [0u32; 3];
        let mut clock = 0;
        let mut now = 0.0;
        let mut look = |heat: &mut Heat, values: &[u32], moved: bool| {
            if moved {
                clock += 100;
            }
            now += 10.0;
            heat.observe(0, clock, now, values);
        };
        look(&mut heat, &values, true);
        assert_eq!(heat.heat(1), 0.0, "nothing changed yet");
        values[1] = 7;
        look(&mut heat, &values, true);
        assert_eq!(heat.heat(1), 1.0);
        assert_eq!(heat.heat(0), 0.0);
        // paused: drawn again, the same colours
        look(&mut heat, &values, false);
        assert_eq!(heat.heat(1), 1.0);
        look(&mut heat, &values, true);
        assert!(heat.heat(1) > 0.0 && heat.heat(1) < 1.0);
        for _ in 0..MOVES {
            look(&mut heat, &values, true);
        }
        assert_eq!(heat.heat(1), 0.0, "cooled down");
        assert!(!heat.fading());
    }

    /// A paused machine's last change stays lit without asking to be drawn again and again.
    #[test]
    fn a_paused_change_does_not_keep_redrawing() {
        let mut heat = Heat::default();
        heat.observe(0, 0, 0.0, &[1]);
        heat.observe(0, 10, 0.0, &[2]);
        // paused at once: the clock stays, the time goes on
        heat.observe(0, 10, 5.0, &[2]);
        assert_eq!(heat.heat(0), 1.0, "still lit");
        assert!(!heat.fading(), "nothing more to show as time passes");
        // running: the moves go by fast, the time dims it
        let mut heat = Heat::default();
        heat.observe(0, 0, 0.0, &[1]);
        heat.observe(0, 10, 0.0, &[2]);
        for frame in 2..8 {
            heat.observe(0, 10 * frame, 0.02 * frame as f64, &[2]);
        }
        assert!(heat.fading(), "{}", heat.heat(0));
    }

    /// While running, a change stays lit for a while, however many frames go by.
    #[test]
    fn a_change_stays_lit_a_second_while_running() {
        let mut heat = Heat::default();
        heat.observe(0, 0, 0.0, &[1]);
        heat.observe(0, 10, 0.02, &[2]);
        for frame in 2..40 {
            heat.observe(0, 10 * frame, 0.02 * frame as f64, &[2]);
        }
        assert!(heat.heat(0) > 0.0, "{}", heat.heat(0));
        heat.observe(0, 1000, 2.0, &[2]);
        assert_eq!(heat.heat(0), 0.0);
    }

    /// Values that come and go (the ROM paged in, out, in again): each compared with itself.
    #[test]
    fn values_that_come_and_go_are_compared_with_themselves() {
        let mut heats = Heats::default();
        for frame in 0..10u64 {
            let (key, value) = if frame % 2 == 0 { (1, 0xc3) } else { (2, 0x00) };
            heats.of(key).observe(key, frame, frame as f64, &[value]);
            assert_eq!(heats.of(key).heat(0), 0.0, "frame {frame}");
        }
    }

    /// Other values (the window shows another address): none lit.
    #[test]
    fn other_values_start_cold() {
        let mut heat = Heat::default();
        heat.observe(0x4000, 0, 0.0, &[1, 2]);
        heat.observe(0xc000, 1, 0.1, &[3, 4]);
        assert_eq!(heat.heat(0), 0.0);
    }
}
