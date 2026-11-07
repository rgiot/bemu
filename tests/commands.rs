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
