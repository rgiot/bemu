//! The commands of the menus and the palette on a running machine: the machine changes and
//! keeps its state (CRTC counters, RAM, CPU), the menu states follow.

use amsrust::commands::{
    self, state, Arg, Command, CommandBus, CommandError, CommandTarget, HostRequest, HostSettings,
    Outcome, Speed,
};
use amsrust::crtc::CrtcType;
use amsrust::debug_server::DebugServer;
use amsrust::motherboard::{CpcModel, Motherboard};

/// A host with settings and a debugger, that records what it is asked.
struct Host {
    motherboard: Motherboard,
    debugger: DebugServer,
    settings: HostSettings,
    requests: Vec<HostRequest>,
}

impl Host {
    fn new() -> Self {
        let mut motherboard = Motherboard::default();
        // to BASIC's prompt
        while motherboard.t_state_counter().value() < 4_000_000 {
            motherboard.execute_next_opcode_no_video();
        }
        Self {
            motherboard,
            debugger: DebugServer::offline(),
            settings: HostSettings::default(),
            requests: Vec::new(),
        }
    }

    fn run(&mut self, id: &str) -> Result<Outcome, CommandError> {
        Command::new(id)?.execute(self)
    }

    fn run_with(&mut self, id: &str, arg: Arg) -> Result<Outcome, CommandError> {
        Command::with_arg(id, arg)?.execute(self)
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n * 19_968 / 4 {
            self.motherboard.execute_next_opcode_no_video();
        }
    }
}

impl CommandTarget for Host {
    fn motherboard(&self) -> &Motherboard {
        &self.motherboard
    }
    fn motherboard_mut(&mut self) -> &mut Motherboard {
        &mut self.motherboard
    }
    fn debugger(&self) -> Option<&DebugServer> {
        Some(&self.debugger)
    }
    fn debugger_mut(&mut self) -> Option<&mut DebugServer> {
        Some(&mut self.debugger)
    }
    fn debugger_and_motherboard(&mut self) -> Option<(&mut DebugServer, &mut Motherboard)> {
        Some((&mut self.debugger, &mut self.motherboard))
    }
    fn settings(&self) -> Option<&HostSettings> {
        Some(&self.settings)
    }
    fn settings_mut(&mut self) -> Option<&mut HostSettings> {
        Some(&mut self.settings)
    }
    fn request(&mut self, request: HostRequest) -> Result<(), CommandError> {
        self.requests.push(request);
        Ok(())
    }
    fn can(&self, _: &HostRequest) -> bool {
        true
    }
}

#[test]
fn a_new_crtc_takes_over_the_registers_and_counters() {
    let mut host = Host::new();
    host.frames(1);
    let before = host.motherboard.crtc().counters();
    let registers = host.motherboard.crtc().registers();
    host.run("machine-crtc-1").unwrap();
    let crtc = host.motherboard.crtc();
    assert_eq!(crtc.crtc_type(), CrtcType::Crtc1);
    assert_eq!(crtc.counters(), before);
    // as the new chip stores them (a CRTC 1 has no VSYNC width in R3)
    for r in [0, 1, 2, 4, 5, 6, 7, 9, 12, 13] {
        assert_eq!(crtc.registers()[r], registers[r], "R{r}");
    }
    assert_eq!(crtc.registers()[3], registers[3] & 0x0f);
    assert!(state("machine-crtc-1", &host).checked);
    assert!(!state("machine-crtc-0", &host).checked);
    // and the frame goes on
    host.frames(2);
    assert_eq!(host.motherboard.crtc().crtc_type(), CrtcType::Crtc1);
}

#[test]
fn the_ram_keeps_its_content_when_it_grows_and_shrinks() {
    let mut host = Host::new();
    host.motherboard.write_bank_byte(4, 0x10, 0xa5); // the first extra bank
    host.run("machine-ram-576").unwrap();
    assert!(state("machine-ram-576", &host).checked);
    assert_eq!(host.motherboard.extra_ram_pages(), 8);
    host.motherboard.write_bank_byte(30, 0x20, 0x5a);
    assert_eq!(host.motherboard.read_bank_byte(4, 0x10), Some(0xa5));
    host.run("machine-ram-default").unwrap();
    assert!(state("machine-ram-default", &host).checked);
    assert_eq!(host.motherboard.read_bank_byte(4, 0x10), Some(0xa5));
}

#[test]
fn cards_are_plugged_and_unplugged_while_it_runs() {
    let mut host = Host::new();
    host.motherboard.write_byte(0x9000, 0x42);
    let pc = host.motherboard.pc();
    host.run("expansion-playcity-toggle").unwrap();
    assert!(state("expansion-playcity-toggle", &host).checked);
    assert_eq!(host.motherboard.pc(), pc);
    host.frames(1);
    host.run("expansion-playcity-toggle").unwrap();
    assert!(!state("expansion-playcity-toggle", &host).checked);
    assert_eq!(host.motherboard.read_byte(0x9000), 0x42);
}

#[test]
fn a_cpc_plus_keeps_the_ram_and_gives_back_its_crtc() {
    let mut host = Host::new();
    host.run("machine-crtc-2").unwrap();
    host.motherboard.write_byte(0x9000, 0x42);
    host.run("machine-platform-cpc6128plus").unwrap();
    assert_eq!(host.motherboard.model(), CpcModel::Cpc6128Plus);
    assert_eq!(host.motherboard.crtc().crtc_type(), CrtcType::Crtc3);
    assert!(!state("machine-crtc-1", &host).enabled);
    assert_eq!(host.motherboard.read_byte(0x9000), 0x42);
    host.run("machine-platform-cpc6128").unwrap();
    assert_eq!(host.motherboard.crtc().crtc_type(), CrtcType::Crtc2);
    assert_eq!(host.motherboard.read_byte(0x9000), 0x42);
}

#[test]
fn shortcuts_pick_the_command_of_the_moment() {
    let mut host = Host::new();
    let f10 = commands::Chord::parse("F10").unwrap();
    assert_eq!(commands::shortcut_command(f10, &host), Some("speed-max"));
    host.run("speed-max").unwrap();
    assert_eq!(host.settings.speed, Speed::Max);
    assert_eq!(commands::shortcut_command(f10, &host), Some("speed-normal"));
    let f8 = commands::Chord::parse("F8").unwrap();
    assert_eq!(commands::shortcut_command(f8, &host), Some("debug-pause"));
    host.run("debug-pause").unwrap();
    assert!(host.debugger.paused());
    assert_eq!(commands::shortcut_command(f8, &host), Some("debug-run"));
}

#[test]
fn input_commands_ask_their_argument() {
    let mut host = Host::new();
    let mut bus = CommandBus::default();
    bus.push_id("debug-set-pc");
    bus.run(&mut host);
    assert_eq!(bus.prompt().unwrap().id, "debug-set-pc");
    bus.answer(Arg::Text("&4000".into()));
    bus.run(&mut host);
    assert_eq!(host.motherboard.pc(), 0x4000);
    assert_eq!(bus.recent().next(), Some("debug-set-pc"));
}

#[test]
fn breakpoints_and_steps_go_through_the_debugger() {
    let mut host = Host::new();
    host.run_with("memory-poke", Arg::Text("&8000 00 00 CD 00 90 00".into()))
        .unwrap();
    host.run_with("memory-poke", Arg::Text("&9000 C9".into()))
        .unwrap();
    host.run_with("debug-set-pc", Arg::Text("&8000".into()))
        .unwrap();
    host.run_with("breakpoint-add", Arg::Text("&8005".into()))
        .unwrap();
    assert!(state("breakpoints-clear", &host).enabled);
    let mut steps = 0;
    while !host.debugger.paused() && steps < 1000 {
        host.motherboard.execute_next_opcode_no_video();
        host.debugger.after_instruction(&mut host.motherboard);
        steps += 1;
    }
    assert_eq!(host.motherboard.pc(), 0x8005);
    host.run("breakpoints-clear").unwrap();
    assert!(!state("breakpoints-clear", &host).enabled);
}

#[test]
fn a_configuration_round_trips() {
    let mut host = Host::new();
    host.run("machine-crtc-1").unwrap();
    host.run("video-rendering-ctm").unwrap();
    let file = std::env::temp_dir().join(format!("bemu-config-{}.json", std::process::id()));
    host.run_with("config-save", Arg::Path(file.clone()))
        .unwrap();
    let saved = std::fs::read_to_string(&file).unwrap();
    let _ = std::fs::remove_file(&file);
    assert!(saved.contains("machine-crtc-1"), "{saved}");
    host.run("config-reset").unwrap();
    assert_eq!(host.motherboard.crtc().crtc_type(), CrtcType::Crtc0);
    host.run_with(
        "config-load",
        Arg::Bytes {
            name: "c.json".into(),
            data: saved.into_bytes(),
        },
    )
    .unwrap();
    assert_eq!(host.motherboard.crtc().crtc_type(), CrtcType::Crtc1);
}

/// Run until the debugger pauses (at most `limit` instructions).
fn run_until_paused(host: &mut Host, limit: usize) {
    for _ in 0..limit {
        if host.debugger.paused() {
            return;
        }
        host.motherboard.execute_next_opcode_no_video();
        host.debugger.after_instruction(&mut host.motherboard);
    }
}

#[test]
fn step_over_a_firmware_rst_returns_after_its_address() {
    let mut host = Host::new();
    // RST &18 (FAR CALL) &9010 ; NOP   with at &9010 a far address block to &9020: RET
    host.run_with("memory-poke", Arg::Text("&8000 DF 10 90 00".into()))
        .unwrap();
    host.run_with("memory-poke", Arg::Text("&9010 20 90 00".into()))
        .unwrap();
    host.run_with("memory-poke", Arg::Text("&9020 C9".into()))
        .unwrap();
    host.run_with("debug-set-pc", Arg::Text("&8000".into()))
        .unwrap();
    host.debugger.set_paused(true);
    host.run("debug-step-over").unwrap();
    run_until_paused(&mut host, 100_000);
    assert_eq!(host.motherboard.pc(), 0x8003);
}

#[test]
fn step_out_stops_when_the_routine_returns_even_at_the_top_of_the_stack() {
    let mut host = Host::new();
    // SP at &0000: the return address at &FFFE, RET makes SP wrap to &0000
    host.run_with("memory-poke", Arg::Text("&8000 CD 00 90 00".into()))
        .unwrap();
    host.run_with("memory-poke", Arg::Text("&9000 00 00 C9".into()))
        .unwrap();
    host.run_with("z80-set-register", Arg::Text("SP=0".into()))
        .unwrap();
    host.run_with("debug-set-pc", Arg::Text("&8000".into()))
        .unwrap();
    host.motherboard.execute_next_opcode_no_video(); // the CALL
    assert_eq!(host.motherboard.pc(), 0x9000);
    host.debugger.set_paused(true);
    host.run("debug-step-out").unwrap();
    run_until_paused(&mut host, 1000);
    assert_eq!(host.motherboard.pc(), 0x8003);
}

#[test]
fn a_stop_cancels_the_step_in_progress() {
    let mut host = Host::new();
    host.run_with("memory-poke", Arg::Text("&8000 00 00 00 00 18 FA".into()))
        .unwrap();
    host.run_with("debug-set-pc", Arg::Text("&8000".into()))
        .unwrap();
    host.run_with("debug-run-to-address", Arg::Text("&7000".into()))
        .unwrap();
    host.run("debug-pause").unwrap();
    assert!(!host.debugger.is_active(), "the run to &7000 is forgotten");
}

#[test]
fn a_model_change_keeps_what_was_chosen_beyond_the_model() {
    let mut host = Host::new();
    host.run("machine-ram-576").unwrap();
    host.run("machine-disc-interface-none").unwrap();
    host.run("machine-platform-cpc464").unwrap();
    assert_eq!(host.motherboard.total_ram_kb(), 576);
    assert!(!host.motherboard.has_disc_interface());
    host.run("machine-platform-cpc6128").unwrap();
    assert_eq!(host.motherboard.total_ram_kb(), 576);
    // the model's own: a 6128 gives a 464 its 64K, and no DDI-1 is invented
    let mut host = Host::new();
    host.run("machine-platform-cpc464").unwrap();
    assert_eq!(host.motherboard.total_ram_kb(), 64);
    assert!(!host.motherboard.has_disc_interface());
    assert!(!state("machine-ram-64", &host).checked);
    host.run("machine-platform-cpc6128").unwrap();
    assert!(
        !state("machine-ram-64", &host).enabled,
        "a 6128 has its own 128K"
    );
}

#[test]
fn a_configuration_runs_only_its_own_commands() {
    let mut host = Host::new();
    let config = r#"{ "crtc": "exit", "rendering": "video-rendering-ctm" }"#;
    let notes = match host.run_with(
        "config-load",
        Arg::Bytes {
            name: "c.json".into(),
            data: config.as_bytes().to_vec(),
        },
    ) {
        Ok(Outcome::Done(Some(message))) => message,
        other => panic!("{other:?}"),
    };
    assert!(
        notes.contains("crtc: exit is not one of its values"),
        "{notes}"
    );
    assert!(host.requests.is_empty(), "exit was not run");
    assert!(state("video-rendering-ctm", &host).checked);
}

#[test]
fn a_drive_becomes_a_gotek() {
    use amsrust::fdc::{DriveKind, FdcDrive};
    let mut host = Host::new();
    host.run("drive-a-gotek").unwrap();
    assert!(state("drive-a-gotek", &host).checked);
    assert!(!state("drive-a-original", &host).checked);
    let kind = host.motherboard.fdc().drive_kind(FdcDrive::A);
    assert_eq!(kind, DriveKind::Gotek);
    // both heads, no head to move
    assert!(kind.is_double_sided());
    assert_eq!(kind.seek_us(40, 3_000), 1);
    assert!(DriveKind::ThreeAndHalfInch.seek_us(40, 3_000) > 100_000);
    assert_eq!("gotek".parse::<DriveKind>(), Ok(DriveKind::Gotek));
}

#[test]
fn a_breakpoint_is_toggled_and_removed() {
    let mut host = Host::new();
    host.run_with("breakpoint-toggle", Arg::Number(0x4000))
        .unwrap();
    assert!(host.debugger.has_exec_breakpoint(0x4000, &host.motherboard));
    assert_eq!(
        host.debugger.exec_breakpoints_mapped(&host.motherboard),
        vec![0x4000]
    );
    host.run_with("breakpoint-toggle", Arg::Number(0x4000))
        .unwrap();
    assert!(!host.debugger.has_exec_breakpoint(0x4000, &host.motherboard));
    host.run_with("breakpoint-add", Arg::Text("&4100".into()))
        .unwrap();
    host.run_with("breakpoint-remove", Arg::Text("&4100".into()))
        .unwrap();
    assert_eq!(host.debugger.exec_breakpoint_count(), 0);
    assert!(host
        .run_with("breakpoint-remove", Arg::Text("&4100".into()))
        .is_err());
    // a memory breakpoint, removed by its number
    host.run_with(
        "breakpoint-add-advanced",
        Arg::Text(r#"{"kind":"mem","access":"w","addr":49152}"#.into()),
    )
    .unwrap();
    host.run_with("breakpoint-remove", Arg::Text("#0".into()))
        .unwrap();
    assert!(!host.debugger.has_breakpoints());
}

#[test]
fn a_register_and_a_flag_are_set_from_the_window() {
    let mut host = Host::new();
    host.run_with("z80-set-register", Arg::Text("HL=49152".into()))
        .unwrap();
    assert_eq!(host.motherboard.hl(), 0xc000);
    let af = host.motherboard.af();
    host.run_with("z80-set-register", Arg::Text(format!("AF={}", af | 1)))
        .unwrap();
    assert_eq!(host.motherboard.af() & 1, 1);
    host.run_with("z80-set-register", Arg::Text("I=63".into()))
        .unwrap();
}

#[test]
fn a_breakpoint_of_the_manager_is_accepted() {
    // what the Breakpoints window's manager posts (cpclib-dap's ManagerBreakpoint::body)
    let mut host = Host::new();
    let body = r#"{"id":0,"label":"ink","kind":"io","access":"w","run":"stop","addr":16384,
        "mask":49152,"size":1,"value":64,"valmask":192,"condition":"A=1","after":2,"name":"ink"}"#;
    host.run_with("breakpoint-add-advanced", Arg::Text(body.into()))
        .unwrap();
    assert!(host.debugger.has_breakpoints());
    host.run_with("breakpoint-add-raster", Arg::Text("156,0".into()))
        .unwrap();
}

#[test]
fn a_typed_backslash_n_is_return() {
    // as typed in the palette: `\n` written with a backslash
    let mut host = Host::new();
    host.run_with("autotype-text", Arg::Text(r"POKE &9000,42\n".into()))
        .unwrap();
    for _ in 0..40_000_000 {
        if !host.debugger.is_typing() {
            break;
        }
        host.motherboard.execute_next_opcode_no_video();
        host.debugger.after_instruction(&mut host.motherboard);
    }
    host.frames(10);
    assert_eq!(host.motherboard.read_byte(0x9000), 42);
}

#[test]
fn a_pen_gets_an_ink_and_the_screen_a_mode() {
    let mut host = Host::new();
    host.run_with("ga-set-ink", Arg::Text("3 6".into()))
        .unwrap();
    host.run_with("ga-set-ink", Arg::Text("B 26".into()))
        .unwrap();
    assert_eq!(host.motherboard.ga().palette()[3].number(), 6);
    assert_eq!(host.motherboard.ga().palette()[16].number(), 26);
    assert!(host
        .run_with("ga-set-ink", Arg::Text("17 0".into()))
        .is_err());
    assert!(host
        .run_with("ga-set-ink", Arg::Text("1 27".into()))
        .is_err());
    host.run_with("ga-set-mode", Arg::Text("2".into())).unwrap();
    // from the next line (the firmware puts its own back at its next interrupt)
    assert_eq!(host.motherboard.ga().next_video_mode() as u8, 2);
}
