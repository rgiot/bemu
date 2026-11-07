//! The host keyboard through the firmware: what BASIC shows for keys as a person presses them,
//! Shift down before the key (src/input.rs's HostKeyboard, a frame at a time).

use amsrust::input::{HostInput, HostKey, HostKeyboard, KeyboardMapping};
use amsrust::keyboard::KeyboardLayout;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};

/// One frame of the window: the keys held, the characters typed.
type Frame = (Vec<HostKey>, Vec<char>);

fn frame(motherboard: &mut Motherboard) {
    let end = motherboard.t_state_counter().value() + 80_000;
    while motherboard.t_state_counter().value() < end {
        motherboard.execute_next_opcode_no_video();
    }
}

/// The screen once BASIC is up and `frames` have been played.
fn screen_after(frames: &[Frame]) -> Vec<u8> {
    screen_after_on(KeyboardLayout::Uk, frames)
}

/// [`screen_after`] on a 6128 of `layout` (`--french`: the French ROMs, the AZERTY keyboard).
fn screen_after_on(layout: KeyboardLayout, frames: &[Frame]) -> Vec<u8> {
    screen_after_with(layout, KeyboardMapping::Symbolic, frames)
}

/// [`screen_after_on`] with the host keyboard's `mapping`.
fn screen_after_with(
    layout: KeyboardLayout,
    mapping: KeyboardMapping,
    frames: &[Frame],
) -> Vec<u8> {
    let mut motherboard = MotherboardBuilder::default()
        .set_keyboard_layout(layout)
        .build();
    for _ in 0..150 {
        frame(&mut motherboard);
    }
    let mut keyboard = HostKeyboard::new(mapping);
    for (down, typed) in frames {
        keyboard.apply(
            &mut motherboard,
            &HostInput {
                down: down.clone(),
                typed: typed.clone(),
                ..Default::default()
            },
        );
        frame(&mut motherboard);
    }
    for _ in 0..10 {
        keyboard.apply(&mut motherboard, &HostInput::default());
        frame(&mut motherboard);
    }
    (0xc000..=0xffffu16)
        .map(|address| motherboard.read_byte(address))
        .collect()
}

/// Shift held alone a frame, then with `key` typing `c`, then released.
fn shifted(key: HostKey, c: char) -> Vec<Frame> {
    let mut frames = vec![(vec![HostKey::LeftShift], vec![])];
    frames.push((vec![HostKey::LeftShift, key], vec![c]));
    frames.extend(std::iter::repeat_n(
        (vec![HostKey::LeftShift, key], vec![]),
        8,
    ));
    frames.push((vec![HostKey::LeftShift], vec![]));
    frames
}

/// An AZERTY host types 1 with Shift+&: BASIC shows 1, not the UK CPC's Shift+1 (!).
#[test]
fn an_azerty_shift_and_ampersand_types_1() {
    let mut plain_1 = vec![(vec![HostKey::Key1], vec!['1'])];
    plain_1.extend(std::iter::repeat_n((vec![HostKey::Key1], vec![]), 8));
    let one = screen_after(&plain_1);
    assert!(
        screen_after(&shifted(HostKey::Key1, '1')) == one,
        "Shift+& typed !, not 1"
    );
    assert!(
        screen_after(&shifted(HostKey::Key1, '!')) != one,
        "a UK host's Shift+1 is still !"
    );
}

/// The same on a French CPC (`--french`): its digits are shifted too, so Shift+& is 1 as it is.
#[test]
fn on_a_french_cpc_shift_and_ampersand_types_1() {
    let french = KeyboardLayout::French;
    // 1 as the CPC's own keys type it (Shift and its & key), and & alone
    let typed = |c: char| vec![(vec![], vec![c])];
    let one = screen_after_on(french, &typed('1'));
    assert!(
        screen_after_on(french, &shifted(HostKey::Key1, '1')) == one,
        "Shift+& is 1 on a French CPC"
    );
    assert!(screen_after_on(french, &typed('&')) != one);
}

/// A key typed as a person types it: down for a few frames with its character, then up.
fn key(host: HostKey, c: Option<char>) -> Vec<Frame> {
    let mut frames = vec![(vec![host], c.into_iter().collect())];
    frames.extend(std::iter::repeat_n((vec![host], vec![]), 4));
    frames.extend(std::iter::repeat_n((vec![], vec![]), 3));
    frames
}

/// The cursor keys move BASIC's cursor: `ab`, left, `x` is the line `axb`.
#[test]
fn the_cursor_keys_move_the_cursor() {
    use HostKey::*;
    let line = |keys: &[(HostKey, Option<char>)]| -> Vec<Frame> {
        keys.iter()
            .flat_map(|&(host, c)| key(host, c))
            .chain(key(Enter, None))
            .collect()
    };
    let typed = line(&[(A, Some('a')), (X, Some('x')), (B, Some('b'))]);
    let edited = line(&[(A, Some('a')), (B, Some('b')), (Left, None), (X, Some('x'))]);
    for mapping in [KeyboardMapping::Symbolic, KeyboardMapping::Positional] {
        let expected = screen_after_with(KeyboardLayout::Uk, mapping, &typed);
        assert!(
            screen_after_with(KeyboardLayout::Uk, mapping, &edited) == expected,
            "{mapping:?}: left"
        );
    }
}

/// The CPC's way of editing a line on the screen: Shift with the arrows moves the copy cursor,
/// COPY (the host's Alt) copies what is under it. `print 1` copied from the line above runs again.
#[test]
fn shift_and_the_arrows_move_the_copy_cursor() {
    use HostKey::*;
    let print_1: Vec<Frame> = [
        (P, 'p'),
        (R, 'r'),
        (I, 'i'),
        (N, 'n'),
        (T, 't'),
        (Space, ' '),
        (Key1, '1'),
    ]
    .into_iter()
    .flat_map(|(host, c)| key(host, Some(c)))
    .chain(key(Enter, None))
    .collect();
    let twice: Vec<Frame> = print_1.iter().chain(&print_1).cloned().collect();
    // Shift held: up 3 lines (Ready, 1, print 1), then COPY 7 times, then RETURN
    let mut copied = print_1.clone();
    let mut shifted = |host: HostKey| {
        copied.extend(std::iter::repeat_n((vec![LeftShift, host], vec![]), 4));
        copied.extend(std::iter::repeat_n((vec![LeftShift], vec![]), 3));
    };
    for _ in 0..3 {
        shifted(Up);
    }
    copied.extend(std::iter::repeat_n((vec![], vec![]), 3));
    for _ in 0..7 {
        copied.extend(key(LeftAlt, None));
    }
    copied.extend(key(Enter, None));
    for mapping in [KeyboardMapping::Symbolic, KeyboardMapping::Positional] {
        let expected = screen_after_with(KeyboardLayout::Uk, mapping, &twice);
        assert!(
            screen_after_with(KeyboardLayout::Uk, mapping, &copied) == expected,
            "{mapping:?}: copied"
        );
    }
}
