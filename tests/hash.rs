use amsrust::{debug::Monitor, motherboard::MotherboardBuilder};
use cpclib_macros::assemble;


// https://64nops.wordpress.com/2020/12/10/hash-algorithms/
const FNV1_32: &[u8] = assemble!(

	"
	org 0x4000
;; FNV-1 32-bit hash
;;
;; FNV_Prime = &01000193 = %0001 0000 0000 0000 0001 1001 0011
;; FNV_Basis = &811C9DC5
;;
;; The 32-bit MUL (Hash * FNV_Prime) is hard-coded with a few shift-and-adds:
;; H = H + (H << 1) + (H << 4) + (H << 7) + (H << 8) + (H << 24)
;;
;; Size : 106 bytes
;; Speed: 7.5k/s
;;
;; Notes:
;;  This implementation has a clumsy register usage.
;;
;; Input:
;;  IX = data
;;  BC = length
;;
;; Output:
;;  HLIY = 32-bit hash
;;  DE,BC,IX,AF,AF' are modified
;;
fnv1_32:
    exx
    ; HLIY (r) = offset_basis
    ld hl,&811C
    ld iy,&9DC5
    exx
_fnv1_32_loop
        exx
        ; DEBC (h) = HLIY (r) = current hash value
        ld d,h
        ld e,l
        ld b,iyh
        ld c,iyl
        ; r += (h<<24)
        ld a,c
        add a,d
        ld h,a
        ; h = h<<1
        sla c
        rl b
        rl e
        rl d
        ; r += (h<<1)
        add iy,bc
        adc hl,de
        ; h = h<<2
        sla c
        rl b
        ex de,hl
        adc hl,hl
        ; h = h<<3
        sla c
        rl b
        adc hl,hl
        ; h = h<<4
        sla c
        rl b
        adc hl,hl
        ex de,hl
        ; r += (h<<4)
        add iy,bc
        adc hl,de
        ; h = h<<5
        sla c
        rl b
        ex de,hl
        adc hl,hl
        ; h = h<<6
        sla c
        rl b
        adc hl,hl
        ; h = h<<7
        sla c
        rl b
        adc hl,hl
        ex de,hl
        ; r += (h<<7)
        add iy,bc
        adc hl,de
        ; h = h<<8
        sla c
        rl b
        rl e
        rl d
        ; r += (h<<8)
        add iy,bc
        adc hl,de

        ; r ^= (data)
        ld a,iyl
        xor (ix+0)
        inc ix
        ld iyl,a
        exx

        ; loop
        dec bc
        ld a,b
        or c
        jr nz,_fnv1_32_loop

    exx
    ret
	"
);


fn test_hash(hash_method: &[u8], data: &[u8], expected: u32) {

	let mut motherboard = MotherboardBuilder::default()
					.disable_lower_rom()
					.disable_upper_rom()
					.build();
	motherboard.write_buffer(0x4000, hash_method);
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

	let obtained = dbg!(dbg!(motherboard.hl()) as u32)*256*256+ motherboard.iy() as u32;
	assert_eq!(
		obtained,
		expected
	);
}

#[test]
fn test_hash_fnv1_32_semilanceata() {
	test_hash(FNV1_32, b"Semilanceata", 0x1E12175C);
}
#[test]
fn test_hash_fnv1_32_longueteau() {
	test_hash(FNV1_32, b"Longueteau", 0x7F7CC956);
}
#[test]
fn test_hash_fnv1_32_severin() {
	test_hash(FNV1_32, b"Severin", 0x9A0DA2E9);
}
#[test]
fn test_hash_fnv1_32_damoiseau() {
	test_hash(FNV1_32, b"Damoiseau", 0x0A5D56CF);
}
  #[test]
  fn test_hash_fnv1_32_foobar() {
  test_hash(FNV1_32, b"foobar", 0x31F0B262);
  }
  #[test]
  fn test_hash_fnv1_32_chongo() {
  test_hash(FNV1_32, b"chongo was here", 0x98A0BF6C);
  }
