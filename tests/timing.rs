use amsrust::{crtc::{Crtc, CrtcType}, debug::Monitor, motherboard::{self, MotherboardBuilder}};
use amsrust::motherboard::Motherboard;
use amsrust::z80::{HasValue, Reg16, Reg8, Z80};
use amsrust::ga::{VideoMode, Sync, RgbLevels};
use itertools::Itertools;

/// Build a standard machine and fill it with the code  and execute it until arriving to jr $
/// Attention, do not put the code in area where ROMs are activated ...
fn execute_instruction(load: u16, test: &[u8]) -> (Vec<(RgbLevels, Sync)> , Motherboard) {
    let mut motherboard = Motherboard::default();

	assert_eq!(
		VideoMode::Mode0,
		motherboard.ga().active_video_mode()
	);
	assert_eq!(
		VideoMode::Mode0,
		motherboard.ga().next_video_mode()
	);

	let res = execute_instruction_in(&mut motherboard, load, test);
	(res, motherboard)
}

fn execute_instruction_in(motherboard: &mut Motherboard, load: u16, test: &[u8]) -> Vec<(RgbLevels, Sync)> {
	dbg!(
		cpclib_asm::disass::disassemble(test)
	);
    motherboard.write_buffer(load, test);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(load);
    motherboard.cpu_mut().reg16_mut(Reg16::Bc).set(2); // always to loops for sensitive instructions

	// execute it until it is finished
	let mut output = Vec::new();
	while motherboard.pc() == load {
		Monitor::monitor(motherboard);
		let mut res = motherboard.execute_next_opcode();
		output.append(&mut res);
	}

	output

}


#[test]
fn timing_nop() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		nop
	"));

	let (output, motherboard) = dbg!(execute_instruction(0x4000, code)); 

	dbg!(output.len(), motherboard.t_state_counter());

	assert_eq!(
		motherboard.cpu().reg16(Reg16::Pc).value(),
		0x4001
	);
	assert_eq!(
		motherboard.t_state_counter().value(),
		1*4
	);

	assert_eq!(
		motherboard.t_state_counter().value()%4,
		0
	);

	assert_eq!(
		motherboard.nops_counter().value(),
		1
	);

	assert_eq!(
		output.len(),
		16
	);
}



#[test]
fn timing_call() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		call 0xdead
	"));

	let (output, motherboard) = dbg!(execute_instruction(0x4000, code)); 

	dbg!(output.len(), motherboard.t_state_counter(), motherboard.nops_counter());

	assert_eq!(
		motherboard.cpu().reg16(Reg16::Pc).value(),
		0xdead
	);

	assert_eq!(
		motherboard.nops_counter().value(),
		5
	);
}



#[test]
fn timing_ldd() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		ldd
	"));

	let (output, motherboard) = execute_instruction(0x4000, code); 

	dbg!(output.len(), motherboard.t_state_counter(), motherboard.nops_counter());

	assert_eq!(
		motherboard.nops_counter().value(),
		5
	);
}



#[test]
fn timing_ldir1() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		ldir
	"));

	let mut motherboard = MotherboardBuilder::default().build();
	motherboard.set_bc(1);
	let output  = execute_instruction_in(&mut motherboard, 0x4000, code);

	dbg!(output.len(), motherboard.t_state_counter(), motherboard.nops_counter());

	assert_eq!(
		motherboard.nops_counter().value(),
		5
	);
}



#[test]
fn timing_ldir2() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		ldir
	"));

	let mut motherboard = MotherboardBuilder::default().build();
	motherboard.set_bc(2);
	let output  = execute_instruction_in(&mut motherboard, 0x4000, code);

	dbg!(output.len(), motherboard.t_state_counter(), motherboard.nops_counter());

	assert_eq!(
		motherboard.nops_counter().value(),
		 6 + 5
	);
}


// Timing based on https://64nops.wordpress.com/2021/01/13/perfectly-accurate-z80-flags-and-cpc-timing/
// Each unique instruction has been modified to reflect a real use
// Several lines has been modified to allow an appropriate parsing

#[test]
fn test_8_load() {
	let madram = "
	; Mnemonic  | Opcodes       | SZ5H3VNC | NOPs  ; Notes
	ld a,b     |    01rrr'''   | ........ | 1    
	ld a,(hl)   |    01rrr110   | ........ | 2 
	ld a,(ix+2) | DD 01rrr110 n | ........ | 5
	ld a,10      |    00rrr110 n | ........ | 2 
	ld (hl),b   |    01110rrr   | ........ | 2
	ld (ix+2),b | DD 01110rrr n | ........ | 5
	ld (hl),4   |    00110110 n | ........ | 3
	ld a,(bc)   |    0A         | ........ | 2
	ld a,(de)   |    1A         | ........ | 2
	ld a,(nn)   |    3A nn      | ........ | 4
	ld (bc),a   |    02         | ........ | 2
	ld (de),a   |    12         | ........ | 2
	ld (nn),a   |    32 nn      | ........ | 4
	ld i,a      | ED 47         | ........ | 3
	ld r,a      | ED 4F         | ........ | 3
	ld a,i      | ED 57         | SZ503!0. | 3 
	ld a,r      | ED 5F         | SZ503!0. | 3";

	check_madram_timing(madram);

}


#[test]
fn test_any() {
	let madram = "
	";
	check_madram_timing(madram);

}

#[test]
fn test_16_load() {
	let madram = "
	ld bc,02    |    00rr0001 nn | ........ | 3 
	ld ix,0xa042   | DD 00100001 nn | ........ | 4
	ld hl,(03)  |    2a nn       | ........ | 5
	ld ix,(03)  | DD 2a nn       | ........ | 6 
	ld bc,(03)  | ED 01rr1011 nn | ........ | 6 
	ld (03),hl  |    22 nn       | ........ | 5
	ld (03),ix  | DD 22 nn       | ........ | 6 
	ld (03),bc  | ED 01rr0011 nn | ........ | 6 
	ld sp,hl    |    F9          | ........ | 2
	ld sp,ix    | DD F9          | ........ | 3
	push bc     |    11qq0101    | ........ | 4 
	push ix     | DD 11100101    | ........ | 5
	pop bc      |    11qq0001    | ........ | 3
	pop ix      | DD 11100001    | ........ | 4";
	check_madram_timing(madram);

}



#[test]
fn test_exchange() {
	let madram = "
	ex de,hl    |    EB | ........ | 1
	ex af,af'   |    08 | ........ | 1
	exx         |    D9 | ........ | 1
	ex (sp),hl  |    E3 | ........ | 6   
	ex (sp),ix  | DD E3 | ........ | 7   

	ldi         | ED A0 | ..!0!!0. | 5   
	ldd         | ED A8 | ..!0!!0. | 5  
	ldir        | ED B0 | ..!0!00. | 11 
	lddr        | ED B8 | ..!0!00. | 11 

	cpi         | ED A1 | SZ!H!!1. | 4   
	cpd         | ED A9 | SZ!H!!1. | 4  
	";

	// not tested yet
	"
	cpir        | ED B1 | SZ!H!!1. | 6 / 4 for last iteration.
	cpdr        | ED B9 | SZ!H!!1. | 6 / 4 for last iteration.
	";
	check_madram_timing(madram);

}

fn check_madram_timing(content: &str) {
	for (code, timing) in content.split("\n")
		.skip(2) // skip header
		.map(|line|{
			let fields = line.split("|").collect_vec();
			(fields[0], fields[3].trim().parse::<usize>().unwrap())
		}) {
			println!("{} {}", code, timing);
			let binary = cpclib_asm::assemble(&format!(" org 0x4000\n {}", code)).unwrap();
			let output  = execute_instruction(0x4000, &binary);
			assert_eq!(
				output.1.nops_counter().value(),
				timing
			);
		}
}