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

#[allow(dead_code)] // assembled (so it keeps assembling), not run yet
const PEARSON_8: &[u8] = assemble!(
    "
;; Pearson hash 8-bit
;;
;; Size : 16 bytes + 256 bytes lookup table
;; Speed: 88.7k/s
;;
;; Input:
;;  IX = data
;;  BC = length
;; Output:
;;  A = 8-bit hash
;;  HL,DE,BC and Flags are modified

pearson8:
	push ix
	pop hl

    ; Adjust 16-bit length for 2x8-bit loops
    inc b
    dec bc
    inc c
    ; LUT pointer
    ld d,_pearson_lut / 256
    ; Initialize hash to zero
    xor a
_pearson8_loop
      xor (hl)
      inc hl
      ld e,a
      ld a,(de)
      dec c
      jr nz,_pearson8_loop
      djnz  _pearson8_loop
    ret


	org 0x5000
_pearson_lut
    DB 98,6,85,150,36,23,112,164,135,207,169,5,26,64,165,219
    DB 61, 20,68,89,130,63,52,102,24,229,132,245,80,216,195,115
    DB 90,168,156,203,177,120,2,190,188,7,100,185,174,243,162,10
    DB 237,18,253,225,8,208,172,244,255,126,101,79,145,235,228,121
    DB 123,251,67,250,161,0,107,97,241,111,181,82,249,33,69,55
    DB 59,153,29,9,213,167,84,93,30,46,94,75,151,114,73,222
    DB 197,96,210,45,16,227,248,202,51,152,252,125,81,206,215,186
    DB 39,158,178,187,131,136,1,49,50,17,141,91,47,129,60,99
    DB 154,35,86,171,105,34,38,200,147,58,77,118,173,246,76,254
    DB 133,232,196,144,198,124,53,4,108,74,223,234,134,230,157,139
    DB 189,205,199,128,176,19,211,236,127,192,231,70,233,88,146,44
    DB 183,201,22,83,13,214,116,109,159,32,95,226,140,220,57,12
    DB 221,31,209,182,143,92,149,184,148,62,113,65,37,27,106,166
    DB 3,14,204,72,21,41,56,66,28,193,40,217,25,54,179,117
    DB 238,87,240,155,180,170,242,212,191,163,78,218,137,194,175,110
    DB 43,119,224,71,122,142,42,160,104,48,247,103,15,11,138,239

	"
);

fn test_hash(hash_method: &[u8], data: &[u8], expected: u32) {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    motherboard.write_buffer(0x4000, hash_method);
    motherboard.write_buffer(0x3000, data);

    let caller = assemble!(
        "
		org 0x1000
		call 0x4000
		jr $
	"
    );
    motherboard.write_buffer(0x1000, caller);

    motherboard.set_pc(0x1000);
    motherboard.set_ix(0x3000);
    motherboard.set_bc(data.len() as _);

    while motherboard.pc() != 0x1000 + 3 {
        Monitor::monitor(&motherboard);
        motherboard.execute_next_opcode();
    }
    Monitor::monitor(&motherboard);

    let obtained = dbg!(dbg!(motherboard.hl()) as u32) * 256 * 256 + motherboard.iy() as u32;
    assert_eq!(obtained, expected);
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
