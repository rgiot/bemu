use amsrust::{debug::Monitor, motherboard::MotherboardBuilder};
use cpclib_macros::assemble;

const CRTC_32: &[u8] = assemble!("

	org 0x4000

;; CRC-32 checksum (unrolled)
;;
;; Poly: &04C11DB7
;; Seed: &FFFFFFFF
;;
;; Input:
;; IX = Data address
;; BC = Data length
;; Output:
;; HLDE = CRC-32
;; IX,BC,AF modified

    MACRO CRC32XOR x1,x2,x3,x4
      rr b
      jr nc,@nextBit
        ld a,e
        xor {x1}
        ld e,a
        ld a,d
        xor {x2}
        ld d,a
        ld a,l
        xor {x3}
        ld l,a
        ld a,h
        xor {x4}
        ld h,a
@nextBit
    MEND

crc32:
    ld hl,&FFFF
    ld d,h
    ld e,l
_crc32_loop
       ld a,(ix+0)
       inc ix
       push bc

       xor e
       ld b,a
       rr b
       jr c,_crc32_bit0
         ld e,d
         ld d,l
         ld l,h
         ld h,0
         jr _crc32_bit1

_crc32_bit0
         ld a,d
         xor &96
         ld e,a
         ld a,l
         xor &30
         ld d,a
         ld a,h
         xor &07
         ld l,a
         ld h,&77

_crc32_bit1
       CRC32XOR &2C,&61,&0E,&EE
       CRC32XOR &19,&C4,&6D,&07
       CRC32XOR &32,&88,&DB,&0E
       CRC32XOR &64,&10,&B7,&1D
       CRC32XOR &C8,&20,&6E,&3B
       CRC32XOR &90,&41,&DC,&76
       CRC32XOR &20,&83,&B8,&ED

       pop bc
       dec bc
       ld a,b
       or c
       jp nz,_crc32_loop

     dec b
     ; Final XOR value
     ; HLDE ^= &FFFFFFFF
     ld a,h
     xor b
     ld h,a
     ld a,l
     xor b
     ld l,a
     ld a,d
     xor b
     ld d,a
     ld a,e
     xor b
     ld e,a

     ret

");

const FLETCHER16: &[u8] = assemble!("
;; Fletcher-16 checksum
;;
;; Input:
;;  HL = Data address
;;  BC = Data length
;; Output:
;;  DE = Fletcher-16
;;  HL,BC,AF are modified

fletcher16:
	push ix
	pop hl
     ; Initialize both sums to zero
     ld de,0
     ; Adjust 16-bit length for 2x8-bit loops
     inc b
     dec bc
     inc c
_fletcher16_loop
       ld a,(hl)
       inc hl
       ; sum1 += data
       add a,e
       ld e,a
       ; sum2 += sum1
       add a,d
       ld d,a
       dec c
       jr nz,_fletcher16_loop
       djnz  _fletcher16_loop
     ret

");


fn test_checksum(checksum_method: &[u8], data: &[u8], expected: u32, thirty32: bool) {

	let mut motherboard = MotherboardBuilder::default()
					.disable_lower_rom()
					.disable_upper_rom()
					.build();
	motherboard.write_buffer(0x4000, checksum_method);
	motherboard.write_buffer(0x3000, data);


	let caller = assemble!("
		org 0x1000
		call 0x4000
		jr $
	");
	motherboard.write_buffer(0x1000, caller);

	motherboard.set_pc(0x1000);
	motherboard.set_ix(0x3000);
	motherboard.set_bc(data.len() as _);

	while motherboard.pc() != 0x1000 + 3 {
		Monitor::monitor(&motherboard);
		motherboard.execute_next_opcode();
	}
	Monitor::monitor(&motherboard);

	let obtained = if thirty32 {
		(motherboard.hl() as u32)*256*256+ motherboard.de() as u32
	} else {
		motherboard.de() as u32
	};
	assert_eq!(
		obtained,
		expected
	);
}


#[test]
fn test_checksum_crc32_semilanceata() {
	test_checksum(CRTC_32, b"Semilanceata", 0xB78816DE, true);
}
#[test]
fn test_checksum_crc32_longueteau() {
	test_checksum(CRTC_32, b"Longueteau", 0x935384D5, true);
}
#[test]
fn test_checksum_crc32_severin() {
	test_checksum(CRTC_32, b"Severin", 0xE442806C, true);
}
#[test]
fn test_checksum_crc32_damoiseau() {
	test_checksum(CRTC_32, b"Damoiseau", 0x95C8BAFA, true);
}




#[test]
fn test_checksum_fletcher_semilanceata() {
	test_checksum(FLETCHER16, b"Semilanceata", 0xB8C7, false);
}
#[test]
fn test_checksum_fletcher_longueteau() {
	test_checksum(FLETCHER16, b"Longueteau", 0x0D19, false);
}
#[test]
fn test_checksum_fletcher_severin() {
	test_checksum(FLETCHER16, b"Severin", 0x1BDC, false);
}
#[test]
fn test_checksum_fletcher_damoiseau() {
	test_checksum(FLETCHER16, b"Damoiseau", 0x4098, false);
}