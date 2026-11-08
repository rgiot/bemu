//! File > Tape > Record: what the firmware's `SAVE` writes on the cassette port, recorded as a
//! CDT, loads back with `LOAD` on another machine.

use amsrust::csl_runner::CslRunner;

/// BASIC 1.0's PRINT token, in the program the 464 keeps from &0170.
const PRINT_TOKEN: u8 = 0xbf;

fn program_has_print(runner: &CslRunner) -> bool {
    // the program is in the first bank of RAM, under the lower ROM
    let ram = |offset: u16| runner.motherboard().read_bank_byte(0, offset).unwrap_or(0);
    (0x0170..0x0200).any(|offset| {
        (0..4).all(|i| ram(offset + i) == [PRINT_TOKEN, 0x20, 0x19, 42][usize::from(i)])
    })
}

#[test]
#[ignore = "slow: boots the firmware, saves and loads a program at 1000 baud"]
fn a_saved_program_loads_back_from_the_recorded_tape() {
    let dir = std::env::temp_dir().join(format!("bemu_tape_record_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut saver = CslRunner::new(&dir);
    saver
        .run_text(
            "cpc_model 0\nreset\nwait 3000000\nkey_output '10 PRINT 42\\(RET)'\nwait 200000\n",
            &dir,
        )
        .unwrap();
    assert!(program_has_print(&saver));
    saver.motherboard_mut().start_tape_recording();
    saver
        .run_text(
            "key_output 'SAVE\"A\"\\(RET)'\nwait 1000000\nkey_output ' '\nwait 15000000\n",
            &dir,
        )
        .unwrap();
    let seconds = saver.motherboard().tape_recorder().unwrap().seconds();
    let cdt = saver.motherboard_mut().stop_tape_recording().unwrap();
    assert!(seconds > 3.0, "{seconds} s recorded");
    std::fs::write(dir.join("a.cdt"), &cdt).unwrap();

    let mut loader = CslRunner::new(&dir);
    loader
        .run_text(
            "cpc_model 0\nreset\nwait 3000000\ntape_insert 'a.cdt'\ntape_play\n\
             key_output 'LOAD\"A\"\\(RET)'\nwait 1000000\nkey_output ' '\nwait 15000000\nscreenshot\n",
            &dir,
        )
        .unwrap();
    assert!(program_has_print(&loader), "the program did not load");
}
