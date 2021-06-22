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
fn test_timing_8_load() {
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
fn test_timing_16_load() {
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
fn test_timing_exchange() {
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
	cpd         | ED A9 | SZ!H!!1. | 4  	";

	// not tested yet
	"
	cpir        | ED B1 | SZ!H!!1. | 6 / 4 for last iteration.
	cpdr        | ED B9 | SZ!H!!1. | 6 / 4 for last iteration.
	";
	check_madram_timing(madram);

}

#[test]
fn test_timing_artihmetic_logic() {
	let madram = "

	add b      |    10000rrr   | SZ5H3V0C | 1 
	add (hl)   |    10000110   | SZ5H3V0C | 2
	add (ix+2) | DD 10000110 n | SZ5H3V0C | 5
	add 3      |    11000110 n | SZ5H3V0C | 2  
	adc b      |    10001rrr   | SZ5H3V0C | 1 
	sub b      |    10010rrr   | SZ5H3V1C | 1 
	sbc b      |    10011rrr   | SZ5H3V1C | 1 
	and b      |    10100rrr   | SZ5H3P00 | 1 
	 or b      |    10101rrr   | SZ5H3P00 | 1 
	xor b      |    10110rrr   | SZ5H3P00 | 1 
	cp  b      |    10111rrr   | SZ5H3V1C | 1 
	inc b      |    00rrr100   | SZ5H3V0. | 1 
	inc (hl)   |    00110100   | SZ5H3V0. | 3
	inc (ix+4) | DD 00110100 n | SZ5H3V0. | 6
	dec b      |    00rrr101   | SZ5H3V1. | 1 
	dec (hl)   |    00110101   | SZ5H3V1. | 3
	dec (ix+5) | DD 00110101 n | SZ5H3V1. | 6";
	check_madram_timing(madram);

}



#[test]
fn test_timing_arithmetic16() {
	let madram = "

	add hl,bc   |    00rr1001 | ..!!!.0C | 3 
add ix,bc   | DD 00rr1001 | ..!!!.0C | 4 ;                 
adc hl,bc   | ED 01rr1010 | SZ!!!V0C | 4 ;                 
sbc hl,bc   | ED 01rr0010 | SZ!!!V1C | 4 ;                 
inc bc      |    00rr0011 | ........ | 2 ;                 
inc bc      | DD 00100011 | ........ | 3 ;                 
dec bc      |    00rr1011 | ........ | 2 ;                 
dec ix      | DD 00101011 | ........ | 3 ;                 
	";
	check_madram_timing(madram);

}


#[test]
fn test_timing_general_purpose() {
	let madram = "

	daa  |    27 | SZ5H3P.C | 1  
	neg  | ED 44 | SZ5H3V1C | 2  
	cpl  |    2F | ..513.1. | 1  
	ccf  |    3F | ..!!!.0C | 1  
	scf  |    37 | ..!0!.01 | 1
	nop  |    00 | ........ | 1  
	halt |    76 | ........ | 1  
	di   |    F3 | ........ | 1  
	ei   |    FB | ........ | 1      
	im 0 | ED 46 | ........ | 2  
	im 1 | ED 56 | ........ | 2 
	im 2 | ED 5E | ........ | 2 ";
	check_madram_timing(madram);

}


#[test]
fn test_timing_rotate() {
	let madram = "

	rlca         |       00000111   | ..503.0C | 1  
	rrca         |       00001111   | ..503.0C | 1  
	rla          |       00010111   | ..503.0C | 1 
	rra          |       00011111   | ..503.0C | 1  
	rlc r        |    CB 00000rrr   | SZ503P0C | 2 
	rlc (hl)     |    CB 00000110   | SZ503P0C | 4
	rlc (ix+2)   | DD CB 00000110 n | SZ503P0C | 7
	rlc (ix+2),b | DD CB 00000rrr n | SZ503P0C | 7 
	rrc r        |    CB 00001rrr   | SZ503P0C | 2  
	rl r         |    CB 00010rrr   | SZ503P0C | 2  
	rr r         |    CB 00011rrr   | SZ503P0C | 2  
	sla r        |    CB 00100rrr   | SZ503P0C | 2  
	sra r        |    CB 00101rrr   | SZ503P0C | 2  
	sl1 r        |    CB 00110rrr   | SZ503P0C | 2  
	srl r        |    CB 00111rrr   | SZ503P0C | 2  
	rld          |    ED 6F         | SZ503P0. | 5 
	rrd          |    ED 67         | SZ503P0. | 5 ";
	check_madram_timing(madram);

}



#[test]
fn test_timing_bit() {
	let madram = "

	bit 2,c        |    CB   01bbbrrr | !Z513Z0. | 2 
	bit 2,(hl)     |    CB   01bbb110 | !Z!1!Z0. | 3
	bit 2,(ix+4)   | DD CB n 01bbb110 | !Z!1!Z0. | 6
	res 2,c        |    CB   10bbbrrr | ........ | 
	res 2,(hl)     |    CB   10bbb110 | ........ | 4
	res 2,(ix+4)   | DD CB n 10bbb110 | ........ | 7
	res 2,(ix+4),c | DD CB n 10bbbrrr | ........ | 7 
	set 2,c        |    CB   11bbbrrr | ........ | 2
	set 2,(hl)     |    CB   11bbb110 | ........ | 4
	set 2,(ix+4)   | DD CB n 11bbb110 | ........ | 7
	set 2,(ix+4),c | DD CB n 11bbbrrr | ........ | 7";
	check_madram_timing(madram);

}


#[test]
fn test_timing_jump() {
	let madram = "

	jp 11       |    11000011 nn  | ........ | 3
	jp c,11   |    11ccc010 nn  | ........ | 3 
	jp hl       |    E9           | ........ | 1 
	jp ix       | DD E9           | ........ | 2	
	jr 1        |    00011000 n-2 | ........ | 3 
	jr c,1    |    001cc000 n-2 | ........ | 3 
	djnz 1      |    00010000 n-2 | ........ | 4 
	call 1     |    11001101 nn  | ........ | 5
	call c,1 |    11ccc100 nn  | ........ | 5 
	rst 0     |    11ttt111     | ........ | 4
	ret         |    11001001     | ........ | 3
	ret c     |    11ccc000     | ........ | 4
	reti        | ED 4D           | ........ | 4 
	retn        | ED 45           | ........ | 4 ";
	check_madram_timing(madram);

}

#[test]
fn test_timing_input() {
	let madram = "

	in a,(2)  |    11011011 n | ........ | 3 
	in b,(C)  | ED 01rrr000   | SZ503P0. | 4 
	in 0,(C)  | ED 01110000   | SZ503P0. | 4
	ini       | ED A2         | !!!!!!!! | 5 
	ind       | ED AA         | !!!!!!!! | 5 
	inir      | ED B2         | !!!!!!!! | 11
	indr      | ED BA         | !!!!!!!! | 11
	out (n),a |    11010011 n | ........ | 3 
	out (C),b | ED 01rrr001   | ........ | 4 
	out (C),0 | ED 01110001   | ........ | 4 
	outi      | ED A3         | !!!!!!!! | 5 
	outd      | ED AB         | !!!!!!!! | 5 
	outir     | ED B3         | !!!!!!!! | 11
	outdr     | ED BB         | !!!!!!!! | 11";
	check_madram_timing(madram);

}

#[test]
fn test_timing_any() {
	let madram = "
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