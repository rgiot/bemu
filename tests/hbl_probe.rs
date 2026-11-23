use amsrust::csl_runner::CslRunner;
#[test]
#[ignore = "slow"]
fn hbl_loads_on_a_3_inch() {
    let mut r = CslRunner::new("/tmp/hbl_fix");
    r.run_text("csl_version 1.0\ncrtc_select 0\nreset\n", ".").unwrap();
    r.motherboard_mut().fdc_mut().start_command_log();
    let mut s = String::from("csl_version 1.0\nwait 3000000\ndisk_insert 'HBL.DSK'\nkey_delay 70000 70000 400000\nwait 500000\nkey_output 'RUN\"HBL\\(RET)'\n");
    for k in 0..30 {
        s += "wait 10000000\n";
        if k >= 6 && k % 2 == 0 { s += "key_output ' '\n"; }
    }
    r.run_text(&s, concat!(env!("CARGO_MANIFEST_DIR"), "/validation/demos/hbl")).unwrap();
    let n = r.motherboard_mut().fdc_mut().command_log().len();
    assert!(n > 800, "the loader stalled after {n} commands");
}
