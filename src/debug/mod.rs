use cpclib_asm::preamble::Token;
use smallvec::SmallVec;
use itertools::Itertools;

use crate::chip::{Chip, ReadAddressChip, ReadDataChip};
use crate::crtc::{Crtc, CrtcPin};
use crate::{ga::*, motherboard};
use crate::motherboard::Motherboard;
use crate::z80::Z80Flags;
use crate::z80::Z80Pin;
use crate::z80::{HasValue, Reg16, Z80};

use self::breakpoint::BreakPoint;


mod interpretor;
mod breakpoint;
mod stepable;

use stepable::StepInto;
use interpretor::*;

use rustyline::error::ReadlineError;
use rustyline::Editor;



pub struct Monitor {}

impl Monitor {
    pub fn monitor(motherboard: &Motherboard) {
        print!("z80: ");
        Monitor::monitor_z80(motherboard.cpu());
        print!("\t");
        Monitor::monitor_z80_flags(motherboard.cpu());
        print!("\t");
        Monitor::monitor_z80_pins(motherboard.cpu());

        print!("stack:\t");
        Monitor::monitor_stack(motherboard, 2, 8);

        print!("ga:\t");
        Monitor::monitor_ga(motherboard.ga());
        print!("\t");
        Monitor::monitor_ga_pins(motherboard.ga());

        print!("crtc:\t");
        Monitor::monitor_crtc_registers(motherboard.crtc());
        print!("\t");
        Monitor::monitor_crtc_counters(motherboard.crtc());
        print!("\t");
        Monitor::monitor_crtc_pins(motherboard.crtc());

        print!("next:\t");
        Monitor::monitor_next_instruction(&motherboard)
    }

    pub fn monitor_crtc(crtc: &Crtc) {
        Monitor::monitor_crtc_counters(crtc);
        Monitor::monitor_crtc_registers(crtc)
    }

    pub fn monitor_crtc_registers(crtc: &Crtc) {
        println!(
            "AR:{} R0:{} R1:{} R2:{} R3:{} R4:{} R5:{} R6:{} R7:{} R8:{} R9:{} R10:{} R11:{} R12:{} R13:{}",
            crtc.ar(),
            crtc.r0(),
            crtc.r1(),
            crtc.r2(),
            crtc.r3(),
            crtc.r4(),
            crtc.r5(),
            crtc.r6(),
            crtc.r7(),
            crtc.r8(),
            crtc.r9(),
            crtc.r10(),
            crtc.r11(),
            crtc.r12(),
            crtc.r13(),
        )
    }

    pub fn monitor_crtc_counters(crtc: &Crtc) {
        println!(
            "VSC:{}{} VCC:{} VLC:{} VTAC:{} | HSC:{}{} HDC:{} HCC:{} | VMA:0x{:x}", 
            if crtc.vsync(){"*"}else{""},
            crtc.vsc,
            crtc.vcc.value(),
            crtc.vlc.value(),
            crtc.vtac,

            if crtc.hsync(){"*"}else{""},
            crtc.hsc,
            crtc.hdc,
            crtc.hcc,

            crtc.vma()
        );
    }

    pub fn monitor_crtc_pins(crtc: &Crtc) {
        println!("CCLK:{} EN:{} A8/RS:{} A9/RW:{} A14/CS:{}, data: {}",
            crtc.get_pin(CrtcPin::CCLK),
            crtc.get_pin(CrtcPin::EN),
            crtc.get_pin(CrtcPin::RS),
            crtc.get_pin(CrtcPin::RW),
            crtc.get_pin(CrtcPin::CS),
            crtc.get_data_from_pins(),
        );
    }

    pub fn monitor_stack(motherboard: &Motherboard, before: u16, after: u16) {
        let sp_orig = motherboard.cpu().reg16(Reg16::Sp).value();
        let sp = sp_orig.wrapping_sub(after * 2);
        let size = after + before;

        (0..size).rev().for_each(|i| {
            let addr = sp.wrapping_add(2 * i);
            let is_top = addr == sp_orig;
            if is_top {
                print!("<<");
            }
            print!("0x{:04X}=0x{:04X}", addr, motherboard.read_word(addr));
            if is_top {
                print!(">> ");
            } else {
                print!(" ");
            }
        });
        println!();
    }

    pub fn monitor_z80(cpu: &Z80) {
        use Reg16::*;
        for reg in [Af, Hl, De, Bc, Ix, Iy, Af2, Hl2, De2, Bc2, Pc, Sp, Wz].into_iter() {
            print!("{:#?}=0x{:04X} ", reg, cpu.reg16(reg).value())
        }
        println!();
    }

    pub fn monitor_z80_pins(cpu: &Z80) {
        println!(
            "MREQ:{} IOREQ:{} WR:{} RD:{} M1:{} RFSH:{} ADR:0x{:x} DATA:{}",
            cpu.get_pin(Z80Pin::Neg_MREQ),
            cpu.get_pin(Z80Pin::Neg_IORQ),
            cpu.get_pin(Z80Pin::Neg_WR),
            cpu.get_pin(Z80Pin::Neg_RD),
            cpu.get_pin(Z80Pin::Neg_M1),
            cpu.get_pin(Z80Pin::Neg_RFSH),
            cpu.get_address_from_pins(),
            cpu.get_data_from_pins()
        );
    }

    pub fn monitor_z80_flags(cpu: &Z80) {   
        if cpu.flag(Z80Flags::S) {
            print!("S");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::Z) {
            print!("Z");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::Y) {
            print!("Y");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::H) {
            print!("H");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::X) {
            print!("X");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::P) {
            print!("P");
        } else {
            print!("-");
        }

        if cpu.flag(Z80Flags::N) {
            print!("N");
        } else {
            print!("-");
        }
      

        if cpu.flag(Z80Flags::C) {
            print!("C");
        } else {
            print!("-");
        }
        println!();
    }

    pub fn monitor_ga(ga: &GateArray) {
        println!(
            "Pen: {:#?} Mode:{} R52: {} UR: {:?}  LR: {:?} Palette {:?}",
            ga.pen().value(),
		    ga.current_mode() as u8,
            ga.r52(),
            ga.upper_rom_pagging(),
            ga.lower_rom_pagging(),
            ga.palette()
        );
    }
    pub fn monitor_ga_pins(cpu: &GateArray) {
        println!(
            "A15:{} A14:{} IORQ:{}",
            cpu.get_pin(GateArrayPin::A15),
            cpu.get_pin(GateArrayPin::A14),
            cpu.get_pin(GateArrayPin::Neg_IORQ),
        );
    }

    pub fn monitor_next_instruction(motherboard: &Motherboard) {
        println!("{}", motherboard.disassemble_next_instruction().to_string());
    }



    /// Wrap the motherboard around a cli interface
    /// TODO handle special case as the one of zexdoc where prints are emulated
    pub fn cli_monitoring(motherboard: &mut Motherboard, ctx: &mut MonitorContext) {
         // `()` can be used when no completer is required
        let mut rl = Editor::<()>::new();
        if rl.load_history("history.txt").is_err() {
            println!("No previous history.");
        }

        let mut previous : Option<String> = None;
        loop {
            ctx.execute_displays(motherboard);
            println!("{}", motherboard.disassemble_next_instruction().to_string());
            let readline = rl.readline(">> ");
            match readline {
                Ok(line) => {
                    // A real TTY hands readline() a line with no trailing
                    // newline; reading from a pipe (a script feeding
                    // commands, or just how this was tested) can still
                    // include it, which would otherwise make the last
                    // "word" e.g. "next\n" instead of "next".
                    let line = line.trim_end().to_owned();

                    let line: String = if line.is_empty() {
                        previous.clone().unwrap_or("h".to_owned())
                    } else {
                        rl.add_history_entry(line.as_str());
                        previous = Some(line.clone());
                        line
                    };
                    
                    let quit = match interpretor::parse_command(&line) {
                        Ok(cmd) => {
                            ctx.execute_command(motherboard, cmd)
                        }
                        _ => {
                            eprintln!("Error in the command line");
                            false
                        }
                    };
                    if quit {
                        break;
                    }
                },
                Err(ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break
                },
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    break
                },
                Err(err) => {
                    println!("Error: {:?}", err);
                    break
                }
            }
        }
        rl.save_history("history.txt").unwrap();
    }
}

pub struct MonitorContext<'f> {
    /// The list of breakpoints to test at each instruction
    breakpoints: Vec<BreakPoint>,
    /// The list of informations to display
    displays: Vec<InfoKind>,

    /// A dedicated code to handle BEFORE the execution of an instruction.
    /// The handler takes as input the motherboard and returns true if emulation need to be stopped
    preExec: Option<Box<&'f mut dyn Fn(&mut Motherboard) -> bool>>,
}

impl<'f> Default for MonitorContext<'f> {
    fn default() -> Self {
        MonitorContext {
            breakpoints: Default::default(),
            displays: Default::default(),
            preExec: None
        }
    }
}

impl<'f> MonitorContext<'f> {
    pub fn new(handle: Box<&'f mut dyn Fn(&mut Motherboard) -> bool>) -> Self {
        Self {
            breakpoints: Default::default(),
            displays: Default::default(),
            preExec: Some(handle)
        }
    }
}
pub enum MonitorError {
}

impl<'f> MonitorContext<'f> {
    /// Execute the command. Return true if it is expected to leave
    pub fn execute_command(&mut self, motherboard: &mut Motherboard, command: Command) -> bool {
        match command {
            Command::Run(addr) => self.execute_run(motherboard, addr),
            Command::Continue => self.execute_continue(motherboard),
            Command::Next => self.execute_next(motherboard),
            Command::Step => self.execute_step(motherboard),

            Command::Peek(addr) => {
                println!("{:x}={:x}", addr, motherboard.read_byte(addr));
                false
            }
            Command::Disassemble(addr) => {
                Self::execute_disassemble(motherboard,  addr);
                false
            },
            Command::Break(b) => {
                self.add_breakpoint(b);
                false
            },
            Command::Clear(b) => {
                self.clear_breakpoint(b);
                false
            }
            Command::Help => {
                Self::execute_help();
                false
            },
            Command::Info(info) => {
                self.execute_info(motherboard, &info); 
                false
            },
            Command::Display(info) => {
                self.displays.push(info);
                false
            }
            Command::Quit => true
        }
    }

    /// Make run the emulator until the next breakpoint. Start from the provided address
    pub fn execute_run(&mut self, motherboard: &mut Motherboard, addr: u16) -> bool {
        motherboard.cpu_mut().set_pc(addr);
        self.execute_continue(motherboard)
    }

    /// Make run the emulator until the next breakpoint. 
    pub fn execute_continue(&mut self, motherboard: &mut Motherboard) -> bool {
        loop {
            let res = self.execute_next(motherboard);
            if res {
                return true;
            }

            if let Some(b) = self.has_some_breakpoint_fired(motherboard) {
                println!("Breakpoint raised: {}", b.to_string());
                break;
            }
        }
        false 
    }

    /// Make run the emulator for a single instruction.
    /// Return true if emulation has to be stopped
    #[inline]
    pub fn execute_next(&mut self, motherboard: &mut Motherboard) -> bool {
        if let Some(handle) = self.preExec.as_mut() {
            let res = handle(motherboard);
            if res {
                return true;
            }
        }
        motherboard.execute_next_opcode_no_video();
        false
    }

    /// Make run the emulator for an instruction plus all its dependencies (complete LDIR, call until its ret, ...)
    pub fn execute_step(&mut self, motherboard: &mut Motherboard) -> bool {
        let next_instruction = motherboard.disassemble_next_instruction();
        if next_instruction.can_use_step_into() {
            let next_pc = motherboard.pc() + next_instruction.len() as u16 ;
            while motherboard.pc() != next_pc {
                let quit = self.execute_next(motherboard);
                if quit {
                    return true;
                }
            }
            false
        }
        else {
            self.execute_next(motherboard)
        }
    }

    /// Check if one breakpoint is fired
    pub fn has_some_breakpoint_fired(&self, motherboard: &mut Motherboard) -> Option<&BreakPoint> {
        self.breakpoints
            .iter()
            .filter(|b| b.is_fired(motherboard))
            .next()
    }

    pub fn execute_info(&self, motherboard: &Motherboard, info: & InfoKind) {
        match info {
            InfoKind::BreakPoint => {
               self.breakpoints.iter()
                .for_each(|b| println!("{}", b.to_string()));
            },
            InfoKind::Crtc => {
                Monitor::monitor_crtc(motherboard.crtc());
            },
            InfoKind::Ga => {
                Monitor::monitor_ga(motherboard.ga());
            },
            InfoKind::Stack => {
                Monitor::monitor_stack(motherboard, 2, 2);
            },
            InfoKind::Z80 => {
                Monitor::monitor_z80(motherboard.cpu());
                Monitor::monitor_z80_flags(motherboard.cpu());
            }
        }
    }

    pub fn execute_displays(&self, motherboard: & Motherboard) {
        self.displays.iter()
            .for_each(|d| self.execute_info(motherboard, d));
    }

    pub fn add_breakpoint(&mut self, b: BreakPoint) {
        self.breakpoints.push(b);
    }

    pub fn clear_breakpoint(&mut self, b: BreakPoint) {
        self.breakpoints.retain(|b| b != b);
    }

    pub fn execute_disassemble(motherboard: & Motherboard, addr: Option<u16>) {
        let addr = addr.unwrap_or(motherboard.cpu().pc());
        let code = motherboard.disassemble_instructions_from(addr, 10);
        for inst in code {
            println!("{}", inst.to_string());
        }
    }

    pub fn execute_help() {
        println!("
        n | next: Execute next instruction
        s | step: Execute next instruction and its consequences
        c | cont | continue: Execute until next breakpoint
        h | help: Display help
        q | quit: Quit monitor
        d | disass | disassemble [addr]: Disassmble at PC or addr if provided
        ");
    }
}



#[derive(Debug)]
pub struct DisassembledInstruction {
    pub token: Token,
    pub address: u16,
    pub memory: SmallVec<[u8; 4]>,
}

impl ToString for DisassembledInstruction {
    fn to_string(&self) -> String {
        format!("0x{:04x} {:<16} {:02x?}", self.address, self.token.to_string(), self.memory)
    }
}

impl DisassembledInstruction {
    /// Number of bytes of the current instruction
    pub fn len(&self) -> usize {
        self.memory.len()
    }
}


