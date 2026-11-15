//! What the disassembly windows draw: branches taken, labels, the arcs of the jumps.

/// A jump, call or return: its target (none for a return or an indirect jump) and, when
/// conditional, whether it is taken with the flags `f`.
pub fn branch(address: u16, bytes: &[u8], f: u8) -> Option<(Option<u16>, Option<bool>)> {
    let op = *bytes.first()?;
    let word = || Some(u16::from_le_bytes([*bytes.get(1)?, *bytes.get(2)?]));
    let relative = || {
        let e = *bytes.get(1)? as i8;
        Some(address.wrapping_add(2).wrapping_add(e as u16))
    };
    // NZ Z NC C PO PE P M
    let holds = |cc: u8| {
        let (mask, set) = match cc {
            0 | 1 => (0x40, cc == 1),
            2 | 3 => (0x01, cc == 3),
            4 | 5 => (0x04, cc == 5),
            _ => (0x80, cc == 7),
        };
        (f & mask != 0) == set
    };
    let cc = (op >> 3) & 7;
    Some(match op {
        0xc3 => (word(), None),
        0xcd => (word(), None),
        0x18 => (relative(), None),
        0x10 => (relative(), None),
        0xc9 => (None, None),
        _ if op & 0xc7 == 0xc2 || op & 0xc7 == 0xc4 => (word(), Some(holds(cc))),
        0x20 | 0x28 | 0x30 | 0x38 => (relative(), Some(holds(cc - 4))),
        _ if op & 0xc7 == 0xc0 => (None, Some(holds(cc))),
        _ => return None,
    })
}

/// `text` (cpclib-dap's: `CALL 0xbb5a`, `LD HL, (0xbe00)`) with the addresses it names written
/// as their labels: the branch's target, and the 16-bit operand of the bytes.
pub fn with_labels(
    text: &str,
    address: u16,
    bytes: &[u8],
    name_at: impl Fn(u16) -> Option<String>,
) -> String {
    let mut candidates: Vec<u16> = Vec::new();
    if let Some((Some(target), _)) = branch(address, bytes, 0) {
        candidates.push(target);
    }
    // a 16-bit operand after the opcode, or after a prefix and the opcode
    for start in [1, 2] {
        if let (Some(&low), Some(&high)) = (bytes.get(start), bytes.get(start + 1)) {
            candidates.push(u16::from_le_bytes([low, high]));
        }
    }
    let mut text = text.to_owned();
    for value in candidates {
        let Some(name) = name_at(value) else { continue };
        let number = format!("0x{value:x}");
        // the number as a whole word only (0xb9 is not the start of 0xb978)
        let mut from = 0;
        while let Some(found) = text[from..].find(&number) {
            let start = from + found;
            let end = start + number.len();
            let alone = !text[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric());
            if alone {
                text.replace_range(start..end, &name);
                break;
            }
            from = end;
        }
    }
    text
}

/// A jump or call drawn in the disassembly's gutter, from row `from` to row `to` (`None`: out
/// of the rows shown, above when `up`), in lane `lane` (0 nearest the instructions).
#[derive(Clone, Debug, PartialEq)]
pub struct Arc {
    pub from: usize,
    pub to: Option<usize>,
    pub up: bool,
    pub lane: usize,
    pub call: bool,
}

/// The arcs of the jumps and calls among `rows` (their address and bytes): the shortest in the
/// lanes nearest the instructions, none sharing a lane where they overlap; at most `lanes`.
pub fn arcs(rows: &[(u16, &[u8])], lanes: usize) -> Vec<Arc> {
    let first = rows.first().map_or(0, |r| r.0);
    let mut found: Vec<(usize, Option<usize>, bool, bool)> = rows
        .iter()
        .enumerate()
        .filter_map(|(index, (address, bytes))| {
            let (target, _) = branch(*address, bytes, 0)?;
            let target = target?;
            let call = matches!(bytes.first(), Some(op) if *op == 0xcd || op & 0xc7 == 0xc4);
            let to = rows.iter().position(|(a, _)| *a == target);
            // a target between two rows (inside an instruction) or off the rows: an arrow out
            let up = target < first || (to.is_none() && target <= *address);
            Some((index, to, up, call))
        })
        .collect();
    // the shortest first: they nest inside the longer ones
    let span = |(from, to, up, _): &(usize, Option<usize>, bool, bool)| match to {
        Some(to) => from.abs_diff(*to),
        None if *up => *from,
        None => rows.len() - from,
    };
    found.sort_by_key(span);
    let mut used: Vec<Vec<(usize, usize)>> = vec![Vec::new(); lanes];
    let mut arcs = Vec::new();
    for (from, to, up, call) in found {
        let (low, high) = match to {
            Some(to) => (from.min(to), from.max(to)),
            None if up => (0, from),
            None => (from, rows.len()),
        };
        let Some(lane) = used
            .iter()
            .position(|taken| taken.iter().all(|&(l, h)| high < l || low > h))
        else {
            continue;
        };
        used[lane].push((low, high));
        arcs.push(Arc {
            from,
            to,
            up,
            lane,
            call,
        });
    }
    arcs
}

/// Draw `arcs` in the gutter `gutter` (its right side touches the instructions), the rows' middles
/// at `rows_y`; the one leaving the PC's row brighter.
pub fn paint_arcs(
    painter: &egui::Painter,
    gutter: egui::Rect,
    rows_y: &[f32],
    arcs: &[Arc],
    pc_row: Option<usize>,
) {
    const LANE: f32 = 6.0;
    for arc in arcs {
        let lit = Some(arc.from) == pc_row;
        let color = match (arc.call, lit) {
            (true, false) => egui::Color32::from_rgb(170, 120, 60),
            (true, true) => egui::Color32::from_rgb(255, 180, 80),
            (false, false) => egui::Color32::from_rgb(80, 130, 190),
            (false, true) => egui::Color32::from_rgb(120, 190, 255),
        };
        let stroke = egui::Stroke::new(if lit { 2.0_f32 } else { 1.0_f32 }, color);
        let right = gutter.right() - 2.0;
        let x = (right - 6.0 - LANE * arc.lane as f32).max(gutter.left() + 1.0);
        let y0 = rows_y[arc.from];
        let y1 = match arc.to {
            Some(to) => rows_y[to],
            None if arc.up => gutter.top(),
            None => gutter.bottom(),
        };
        painter.line_segment([egui::pos2(right, y0), egui::pos2(x, y0)], stroke);
        painter.line_segment([egui::pos2(x, y0), egui::pos2(x, y1)], stroke);
        if arc.to.is_some() {
            painter.line_segment([egui::pos2(x, y1), egui::pos2(right, y1)], stroke);
            // the arrow head on the target
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(right, y1),
                    egui::pos2(right - 4.0, y1 - 3.0),
                    egui::pos2(right - 4.0, y1 + 3.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
        } else {
            // out of the rows shown: an arrow up or down at the edge
            let dy = if arc.up { 4.0 } else { -4.0 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(x, y1),
                    egui::pos2(x - 3.0, y1 + dy),
                    egui::pos2(x + 3.0, y1 + dy),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operands_are_written_as_labels() {
        let name = |a: u16| match a {
            0xbb5a => Some("TXT_OUTPUT".to_owned()),
            0xb9 => Some("short".to_owned()),
            _ => None,
        };
        assert_eq!(
            with_labels("CALL 0xbb5a", 0, &[0xcd, 0x5a, 0xbb], name),
            "CALL TXT_OUTPUT"
        );
        // a number that only begins like a label's is left
        assert_eq!(
            with_labels("LD HL, 0xb978", 0, &[0x21, 0x78, 0xb9], name),
            "LD HL, 0xb978"
        );
        assert_eq!(with_labels("LD A, C", 0, &[0x79], name), "LD A, C");
    }

    #[test]
    fn arcs_take_lanes_without_overlapping() {
        let jr = |e: u8| [0x18, e];
        // row 0: JR to row 2; row 1: JR to row 3 (overlaps the first); row 2: NOP; row 3: JP out
        let a = jr(2);
        let b = jr(1);
        let rows: Vec<(u16, &[u8])> = vec![
            (0x100, &a),
            (0x102, &b),
            (0x104, &[0x00]),
            (0x105, &[0xc3, 0x00, 0x80]),
        ];
        let arcs = arcs(&rows, 4);
        assert_eq!(arcs.len(), 3);
        let to = |from: usize| arcs.iter().find(|a| a.from == from).unwrap();
        assert_eq!(to(0).to, Some(2));
        assert_eq!(to(1).to, Some(3));
        assert_ne!(to(0).lane, to(1).lane);
        // the JP to &8000 leaves the rows downwards
        assert_eq!((to(3).to, to(3).up), (None, false));
    }

    #[test]
    fn a_branch_is_taken_as_the_flags_say() {
        // JR NZ,+2 at &4000: Z clear, taken, to &4004
        assert_eq!(
            branch(0x4000, &[0x20, 0x02], 0x00),
            Some((Some(0x4004), Some(true)))
        );
        assert_eq!(
            branch(0x4000, &[0x20, 0x02], 0x40),
            Some((Some(0x4004), Some(false)))
        );
        // CALL C,&BB5A with carry
        assert_eq!(
            branch(0, &[0xdc, 0x5a, 0xbb], 0x01),
            Some((Some(0xbb5a), Some(true)))
        );
        // RET M, sign clear
        assert_eq!(branch(0, &[0xf8], 0x00), Some((None, Some(false))));
        assert_eq!(branch(0, &[0x3e, 0x01], 0), None);
    }
}
