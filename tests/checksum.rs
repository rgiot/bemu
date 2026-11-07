use amsrust::{debug::Monitor, motherboard::MotherboardBuilder};
use cpclib_macros::assemble;

const CRTC_32: &[u8] = assemble!(
    "

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

crc32:
    ld hl,&FFFF
    ld d,h
    ld e,l
_crc32_loop:
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

_crc32_bit0:
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

      _crc32_bit1:
        rr b
        jr nc,_crc32_nextbit_1
          ld a,e
          xor &2C
          ld e,a
          ld a,d
          xor &61
          ld d,a
          ld a,l
          xor &0E
          ld l,a
          ld a,h
          xor &EE
          ld h,a
      _crc32_nextbit_1:
        rr b
        jr nc,_crc32_nextbit_2
          ld a,e
          xor &19
          ld e,a
          ld a,d
          xor &C4
          ld d,a
          ld a,l
          xor &6D
          ld l,a
          ld a,h
          xor &07
          ld h,a
      _crc32_nextbit_2:
        rr b
        jr nc,_crc32_nextbit_3
          ld a,e
          xor &32
          ld e,a
          ld a,d
          xor &88
          ld d,a
          ld a,l
          xor &DB
          ld l,a
          ld a,h
          xor &0E
          ld h,a
      _crc32_nextbit_3:
        rr b
        jr nc,_crc32_nextbit_4
          ld a,e
          xor &64
          ld e,a
          ld a,d
          xor &10
          ld d,a
          ld a,l
          xor &B7
          ld l,a
          ld a,h
          xor &1D
          ld h,a
      _crc32_nextbit_4:
        rr b
        jr nc,_crc32_nextbit_5
          ld a,e
          xor &C8
          ld e,a
          ld a,d
          xor &20
          ld d,a
          ld a,l
          xor &6E
          ld l,a
          ld a,h
          xor &3B
          ld h,a
      _crc32_nextbit_5:
        rr b
        jr nc,_crc32_nextbit_6
          ld a,e
          xor &90
          ld e,a
          ld a,d
          xor &41
          ld d,a
          ld a,l
          xor &DC
          ld l,a
          ld a,h
          xor &76
          ld h,a
      _crc32_nextbit_6:
        rr b
        jr nc,_crc32_nextbit_7
          ld a,e
          xor &20
          ld e,a
          ld a,d
          xor &83
          ld d,a
          ld a,l
          xor &B8
          ld l,a
          ld a,h
          xor &ED
          ld h,a
      _crc32_nextbit_7:

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

"
);

const FLETCHER16: &[u8] = assemble!(
    "
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
  _fletcher16_loop:
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

"
);

fn test_checksum(checksum_method: &[u8], data: &[u8], expected: u32, thirty32: bool) {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    motherboard.write_buffer(0x4000, checksum_method);
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

    let obtained = if thirty32 {
        (motherboard.hl() as u32) * 256 * 256 + motherboard.de() as u32
    } else {
        motherboard.de() as u32
    };
    assert_eq!(obtained, expected);
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
