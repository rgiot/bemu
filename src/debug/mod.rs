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

pub use self::breakpoint::BreakPoint;


mod interpretor;
mod breakpoint;
mod stepable;

use stepable::StepInto;
use interpretor::*;

#[cfg(not(target_arch = "wasm32"))]
use rustyline::error::ReadlineError;
#[cfg(not(target_arch = "wasm32"))]
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



    /// Wrap the motherboard around a cli interface (a terminal: not in a web page)
    /// TODO handle special case as the one of zexdoc where prints are emulated
    #[cfg(not(target_arch = "wasm32"))]
    pub fn cli_monitoring(motherboard: &mut Motherboard, ctx: &mut MonitorContext) {
         // `()` can be used when no completer is required
        let mut rl = Editor::<()>::new();
        if rl.load_history("history.txt").is_err() {
            println!("No previous history.");
        }

        let mut previous : Option<String> = None;
        loop {
            ctx.execute_displays(motherboard);
            ctx.print_instruction(&motherboard.disassemble_next_instruction());
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
                    
                    let quit = match interpretor::parse_command(&line, &ctx.symbols) {
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

    /// Names of addresses, for the commands and the disassembly.
    symbols: crate::symbols::SymbolTable,
}

impl<'f> Default for MonitorContext<'f> {
    fn default() -> Self {
        MonitorContext {
            breakpoints: Default::default(),
            displays: Default::default(),
            preExec: None,
            symbols: Default::default()
        }
    }
}

impl<'f> MonitorContext<'f> {
    pub fn new(handle: Box<&'f mut dyn Fn(&mut Motherboard) -> bool>) -> Self {
        Self {
            breakpoints: Default::default(),
            displays: Default::default(),
            preExec: Some(handle),
            symbols: Default::default()
        }
    }

    /// Names for addresses (a snapshot's labels, symbol files), added to those known.
    pub fn add_symbols(&mut self, symbols: &crate::symbols::SymbolTable) {
        self.symbols.extend(symbols);
    }

    /// An instruction as the disassembly shows it: under its label, with the symbol its
    /// address operand or jump target names.
    pub fn format_instruction(&self, instruction: &DisassembledInstruction) -> String {
        let mut text = String::new();
        if let Some(label) = self.symbols.name_at(instruction.address) {
            text.push_str(&format!("{label}:\n"));
        }
        text.push_str(&instruction.to_string());
        if let Some(target) = instruction.target().and_then(|target| self.symbols.describe(target)) {
            text.push_str(&format!(" ; {target}"));
        }
        text
    }

    fn describe_breakpoint(&self, breakpoint: &BreakPoint) -> String {
        match breakpoint {
            BreakPoint::Address(address) => match self.symbols.describe(*address) {
                Some(label) => format!("{} ({label})", breakpoint.to_string()),
                None => breakpoint.to_string()
            }
        }
    }

    pub fn print_instruction(&self, instruction: &DisassembledInstruction) {
        println!("{}", self.format_instruction(instruction));
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
                self.execute_disassemble(motherboard,  addr);
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
                println!("Breakpoint raised: {}", self.describe_breakpoint(b));
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
                .for_each(|b| println!("{}", self.describe_breakpoint(b)));
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
        self.breakpoints.retain(|kept| *kept != b);
    }

    pub fn execute_disassemble(&self, motherboard: & Motherboard, addr: Option<u16>) {
        let addr = addr.unwrap_or(motherboard.cpu().pc());
        let code = motherboard.disassemble_instructions_from(addr, 10);
        for inst in code {
            self.print_instruction(&inst);
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

    /// The address the instruction names: a relative jump's target, or a 16-bit operand
    /// (a jump, a call, a load from or to an address, a constant that may be one).
    pub fn target(&self) -> Option<u16> {
        let bytes = &self.memory;
        match (bytes.len(), bytes.first()) {
            (2, Some(0x10 | 0x18 | 0x20 | 0x28 | 0x30 | 0x38)) => {
                Some(self.address.wrapping_add(2).wrapping_add(bytes[1] as i8 as u16))
            },
            (3, Some(&prefix)) if prefix != 0xdd && prefix != 0xfd && prefix != 0xcb => {
                Some(u16::from_le_bytes([bytes[1], bytes[2]]))
            },
            // DD/FD/ED: the operand after the two opcode bytes
            (4, Some(0xdd | 0xfd | 0xed)) if bytes[1] != 0xcb && bytes[1] != 0x36 => {
                Some(u16::from_le_bytes([bytes[2], bytes[3]]))
            },
            _ => None
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_disassembly_shows_the_labels() {
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().disable_lower_rom().build();
        // start: call sub : jr start ; sub: ret
        motherboard.write_buffer(0x4000, &[0xcd, 0x05, 0x40, 0x18, 0xfb, 0xc9]);
        let mut symbols = crate::symbols::SymbolTable::default();
        symbols.insert("start", 0x4000);
        symbols.insert("sub", 0x4005);
        let mut ctx = MonitorContext::default();
        ctx.add_symbols(&symbols);
        let lines: Vec<String> = motherboard
            .disassemble_instructions_from(0x4000, 3)
            .iter()
            .map(|instruction| ctx.format_instruction(instruction))
            .collect();
        assert!(lines[0].starts_with("start:\n") && lines[0].ends_with("; sub"), "{}", lines[0]);
        assert!(lines[1].ends_with("; start"), "{}", lines[1]);
        assert!(lines[2].starts_with("sub:\n"), "{}", lines[2]);
    }
}
