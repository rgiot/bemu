; R0LOWER - CRTC 0: an R0 LOWERED under C0 (21 from 41), on which character is it still not counting on to 255?
;
; Like R0SIDE's mode R with a lowering instead of a raise: R0 (41, R2=34) is written 21 at a time that goes up
; by 1 us a frame, and 41 again 20 us later by an OUT (C),C. A 21 written once C0 has passed it makes C0 count
; on to 255 and wrap (a line of 256 characters: E, about 190 us more in the frame, seen from the passes of a
; 10 us loop to the next VSYNC); written before, the line ends at C0=21; written on C0=41 or later (the end
; already committed to) it is the edge this test is about. Rows of 64 results (the control's passes first, 130
; bytes each, in RAM at #4900 (OUT (C),C), #4A00 (OUTI), #4B00 (OUT (n),A)): E = word >= control + 10.
; Build: basm r0lower.asm (writes R0LOW.BIN on r0lower.dsk)   Run: RUN"R0LOW"

TXT_OUTPUT equ #bb5a
KM_WAIT_CHAR equ #bb06
results equ #4800
tabr equ #5000               ; the delays of mode R: 64 entries of 9 bytes (3 delays: passes lo, hi, remainder)
tabs equ #5400               ; the same for mode S
BASE equ 1888
TOTAL equ 80
MID equ 20

        org #4000
start:
        di
        ld hl,title : call print_str
        call make_tables
        ld bc,#bc02 : out (c),c : ld bc,#bd22 : out (c),c      ; R2 = 34
        ld bc,#bc00 : out (c),c : ld bc,#bd29 : out (c),c      ; R0 = 41
        ld de,#4900 : ld (dest),de
        ld a,0 : ld (mode),a
        ld a,0 : ld (instr),a : call row
        ld a,1 : ld (instr),a : call row
        ld a,2 : ld (instr),a : call row
        ld hl,txt_end : call print_str
        ei
        call KM_WAIT_CHAR
        ret

; one row: the sweep, then its results copied to (dest), dest += #100
row:
        call sweep
        ld hl,results : ld de,(dest) : ld bc,130 : ldir
        ld (dest),de
        ld a,(dest+1) : inc a : ld (dest+1),a
        xor a : ld (dest),a
        ld a,'.' : call TXT_OUTPUT
        ret

; the delays: mode R (BASE+d, MID, TOTAL-d), mode S (BASE, MID+d, TOTAL-d)
make_tables:
        ld ix,tabr
        ld iy,tabs
        ld b,64 : ld c,0
mt_loop:
        push bc
        ld l,c : ld h,0 : ld de,BASE : add hl,de : call divide7
        ld (ix+0),c : ld (ix+1),b : ld (ix+2),a
        ld hl,MID : call divide7
        ld (ix+3),c : ld (ix+4),b : ld (ix+5),a
        pop bc : push bc
        ld a,TOTAL : sub c : ld l,a : ld h,0 : call divide7
        ld (ix+6),c : ld (ix+7),b : ld (ix+8),a
        ld hl,BASE : call divide7
        ld (iy+0),c : ld (iy+1),b : ld (iy+2),a
        pop bc : push bc
        ld l,c : ld h,0 : ld de,MID : add hl,de : call divide7
        ld (iy+3),c : ld (iy+4),b : ld (iy+5),a
        pop bc : push bc
        ld a,TOTAL : sub c : ld l,a : ld h,0 : call divide7
        ld (iy+6),c : ld (iy+7),b : ld (iy+8),a
        ld de,9 : add ix,de : add iy,de
        pop bc
        inc c
        djnz mt_loop
        ret

; BC = HL / 7, A = HL mod 7
divide7:
        ld bc,0 : ld de,7
d7_l:   or a : sbc hl,de
        jr c,d7_r
        inc bc
        jr d7_l
d7_r:   add hl,de
        ld a,l
        ret

; 65 trials: the control (trial 0: the same writes of 41) then d=0..63
sweep:
        xor a : ld (trial),a
        ld hl,results : ld (rptr),hl
sw_next:
        ld a,(trial) : or a : jr z,sw_z : dec a
sw_z:   ld l,a : ld h,0 : ld d,h : ld e,l
        add hl,hl : add hl,hl : add hl,hl : add hl,de           ; x 9
        ld a,(mode) : or a : ld de,tabr : jr z,sw_t : ld de,tabs
sw_t:   add hl,de : ld (entry),hl
        ; the raise and restore routines of this row
        ld a,(trial) : or a : ld a,#29 : jr z,sw_c : ld a,#15
sw_c:   ld (vraise),a
        ld a,(mode) : or a : jr nz,sw_s
        ld a,(instr) : call pick : ld (raise_at),hl : ld hl,wr_c : ld (restore_at),hl : jr sw_go
sw_s:   ld hl,wr_c : ld (raise_at),hl : ld a,(instr) : call pick : ld (restore_at),hl
sw_go:  call wait_vsync
        ld ix,(entry)
        ld c,(ix+0) : ld b,(ix+1) : ld e,(ix+2) : ld d,0
        call delay
        ld a,(vraise) : ld (vsel),a
        ld hl,(raise_at) : ld de,after1 : push de : jp (hl)
after1: ld ix,(entry)
        ld c,(ix+3) : ld b,(ix+4) : ld e,(ix+5) : ld d,0
        call delay
        ld a,#29 : ld (vsel),a
        ld hl,(restore_at) : ld de,after2 : push de : jp (hl)
after2: ld ix,(entry)
        ld c,(ix+6) : ld b,(ix+7) : ld e,(ix+8) : ld d,0
        call delay
        call count_to_vsync
        ld de,(rptr) : ld a,l : ld (de),a : inc de : ld a,h : ld (de),a : inc de : ld (rptr),de
        ld a,(trial) : inc a : ld (trial),a
        cp 65
        jp nz,sw_next
        ret

; HL = the routine writing with the instruction A (0 OUT (C),C, 1 OUTI, 2 OUT (n),A)
pick:   ld hl,wr_c : or a : ret z
        ld hl,wr_i : dec a : ret z
        ld hl,wr_n
        ret

; the writes: the raise writes (vraise) and the restore #29 (vrest, set before): both routines write
; the byte in (vwrite) = (vraise) for a raise, #29 for a restore. They are told apart by the caller:
; the raise routine of a row is entered with vwrite = vraise.
wr_c:   ld a,(vsel) : ld c,a : ld b,#bd : out (c),c : ret
wr_i:   ld hl,vsel : ld bc,#be00 : outi : ret
wr_n:   ld a,(vsel) : out (0),a : ret

; BC passes (at least 1), E remainder
delay:
dl_pass: dec bc : ld a,b : or c : jr nz,dl_pass          ; 7 us a pass
        ld hl,sled_end : ld d,0 : or a : sbc hl,de
        jp (hl)
        nop : nop : nop : nop : nop : nop
sled_end:
        ret

wait_vsync:
        di
        ld b,#f5
wv_out: in a,(c) : rra : jr c,wv_out
wv_in:  in a,(c) : rra : jr nc,wv_in
        ld bc,#bc00 : out (c),c
        ret

count_to_vsync:
        ld hl,0
        ld b,#f5
cv_high: inc hl : in a,(c) : rra : jr c,cv_high
cv_low: inc hl : in a,(c) : rra : jr nc,cv_low
        ret

print_str:
        ld a,(hl) : or a : ret z
        call TXT_OUTPUT
        inc hl
        jr print_str

mode:    db 0
instr:   db 0
trial:   db 0
entry:   dw 0
rptr:    dw 0
dest:    dw 0
raise_at: dw 0
restore_at: dw 0
vraise:  db #15
vrest:   db #29
vsel:    db #15
title:  db "R0LOW (CRTC 0)",13,10,"six rows: see the source",13,10,0
txt_end: db 13,10,"Done: results at #4900..#4E00",13,10,0
prog_end:
        save "R0LOW.BIN", start, prog_end-start, AMSDOS, "r0lower.dsk"
