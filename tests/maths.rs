use amsrust::{debug::Monitor, motherboard::MotherboardBuilder};
use cpclib_macros::assemble;

// http://www.cpctech.cpc-live.com/docs/mult.html
const MUL_DE_BC: &[u8] = assemble!("
;; multiply DE and BC
;; DE is equivalent to the number in the top row in our algorithm
;; and BC is equivalent to the number in the bottom row in our algorithm

    ld a,16     ; this is the number of bits of the number to process
    ld hl,0     ; HL is updated with the partial result, and at the end it will hold 
                ; the final result.
.mul_loop
    srl b
    rr c        ;; divide BC by 2 and shifting the state of bit 0 into the carry
                ;; if carry = 0, then state of bit 0 was 0, (the rightmost digit was 0) 
                ;; if carry = 1, then state of bit 1 was 1. (the rightmost digit was 1)
                ;; if rightmost digit was 0, then the result would be 0, and we do the add.
                ;; if rightmost digit was 1, then the result is DE and we do the add.
    jr nc, .no_add	

    ;; will get to here if carry = 1        
    add hl,de   

.no_add
    ;; at this point BC has already been divided by 2

    ex de,hl    ;; swap DE and HL
    add hl,hl   ;; multiply DE by 2
    ex de,hl    ;; swap DE and HL

    ;; at this point DE has been multiplied by 2
    
    dec a
    jr nz, .mul_loop  ;; process more bits

	ret
");



fn test_multiplication_bc_de(de: u16, bc: u16, expected: u16) {

	let mut motherboard = MotherboardBuilder::default()
					.disable_lower_rom()
					.disable_upper_rom()
					.build();
	motherboard.write_buffer(0x4000, MUL_DE_BC);


	let caller = assemble!("
		org 0x1000
		call 0x4000
		jr $
	");
	motherboard.write_buffer(0x1000, caller);

	motherboard.set_pc(0x1000);
	motherboard.set_de(de);
	motherboard.set_bc(bc);

	while motherboard.pc() != 0x1000 + 3 {
		Monitor::monitor(&motherboard);
		motherboard.execute_next_opcode();
	}
	Monitor::monitor(&motherboard);

	let obtained = motherboard.hl();
	assert_eq!(
		obtained,
		expected
	);
}


#[test]
fn test_multiplication() {
	
	test_multiplication_bc_de(32, 48, 1536);
	test_multiplication_bc_de(1, 1, 1);
	test_multiplication_bc_de(1000, 0, 0);

}