//! The app's touch keyboard (app/src/main/kotlin/org/bemu/droid/CpcKeyboard.kt) sends the
//! firmware's key numbers: each character key's number must be the key that types its label on
//! the UK firmware, so the Kotlin table and amsrust's matrix cannot drift apart.

use amsrust::keyboard::KeyboardLayout;

#[test]
fn the_touch_keys_are_where_the_firmware_types_their_labels() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../app/src/main/kotlin/org/bemu/droid/CpcKeyboard.kt"
    ))
    .unwrap();
    let mut checked = 0;
    // CpcKey("Q", 67) / CpcKey("\\", 22, ...)
    for part in source.split("CpcKey(\"").skip(1) {
        let Some((label, rest)) = part.split_once("\", ") else { continue };
        let label = label.replace("\\\\", "\\");
        let number: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let Ok(number) = number.parse::<usize>() else { continue };
        // the character keys (not the arrows of the cursor keys)
        if label.chars().count() != 1 || !label.chars().all(|c| c.is_ascii_graphic()) {
            continue;
        }
        let (normal, shifted) = KeyboardLayout::Uk
            .legend(number)
            .unwrap_or_else(|| panic!("{label:?}: key {number} types nothing"));
        let c = label.chars().next().unwrap();
        assert!(
            c == normal || c == shifted || c.to_ascii_lowercase() == normal,
            "{label:?} is key {number}, which types {normal:?} / {shifted:?}"
        );
        checked += 1;
    }
    assert!(checked > 40, "{checked} keys checked");
}
