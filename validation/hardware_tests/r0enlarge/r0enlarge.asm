; R0ENLARGE - CRTC 0 hardware test: an R0 written to enlarge a line, on which character does it
; stop being in time?
;
; On a normal frame (R0=63) a line far from the VSYNC gets R0=#7F written at a time that goes up by
; 1 us a frame (64 frames, covering one line), then R0=63 about 11 us later. If the first write came
; in time C0 is past 63 and goes on: R0 lowered under C0 makes it count to 255 and wrap, a line of
; 256 characters, about 190 us more in the frame (measured by counting the 10 us passes to the next
; VSYNC, as R0WRAP does). If it came too late C0 has wrapped already and nothing changes.
; Two rows, one with OUT (C),C (effect on its 3rd us) and one with OUTI (5th us); E = enlarged.
; The position at which E turns into . is the last character on which R0 is taken; the same
; program on two emulators tells where each one stands (the compendium 13.6.1 chronogram: C0=R0 is
; still in time). The time of a trial is the same whatever the delay (the delay before the write and
; the one after it add up to a constant) and nothing is printed between trials, so that the VSYNC
; polling loops always have the same phase.
; Build: basm r0enlarge.asm (writes R0ENL.BIN on r0enlarge.dsk)   Run: RUN"R0ENL"

TXT_OUTPUT equ #bb5a
KM_WAIT_CHAR equ #bb06
BASE equ 1900               ; us from the VSYNC detection to the first delay's end for d=0
TOTAL equ 80                ; the second delay is TOTAL-d us

        org #4000
start:
        di
        ld hl,title : call print_str
        call make_table
        ld hl,txt_a : call print_str
        xor a : ld (variant),a
        call sweep
        call show
        ld hl,txt_b : call print_str
        ld a,1 : ld (variant),a
        call sweep
        call show
        ld hl,txt_end : call print_str
        ei
        call KM_WAIT_CHAR
        ret

; the passes (of 7 us) and the remainder of BASE+d and of TOTAL-d, for d=0..63: 6 bytes each
make_table:
        ld ix,tab
        ld b,64 : ld c,0                 ; c = d
mt_loop:
        push bc
        ld l,c : ld h,0 : ld de,BASE : add hl,de
        call divide7
        ld (ix+0),c : ld (ix+1),b : ld (ix+2),a
        pop bc : push bc
        ld a,TOTAL : sub c : ld l,a : ld h,0
        call divide7
        ld (ix+3),c : ld (ix+4),b : ld (ix+5),a
        ld de,6 : add ix,de
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

; 65 trials: the control (nothing changes, trial 0) then d=0..63
sweep:
        xor a : ld (trial),a
        ld hl,results : ld (rptr),hl
sw_next:
        ld a,(trial) : or a : jr z,sw_z : dec a
sw_z:   ld l,a : ld h,0 : ld d,h : ld e,l
        add hl,hl : add hl,de : add hl,hl : ld de,tab : add hl,de
        ld (entry),hl
        ld a,(trial) : or a : ld a,#3f : jr z,sw_ctl : ld a,#7f
sw_ctl: ld (v1),a
        call wait_vsync
        ld ix,(entry)
        ld c,(ix+0) : ld b,(ix+1) : ld e,(ix+2) : ld d,0
        call delay
        ld a,(variant) : or a : jr nz,sw_outi
        ld a,(v1) : ld c,a : ld b,#bd : out (c),c       ; R0 = #7F (#3F for the control)
        ld a,(v2) : ld c,a : ld b,#bd : out (c),c       ; R0 = 63
        jr sw_done
sw_outi: ld hl,v1 : ld bc,#be00 : outi
        ld hl,v2 : ld bc,#be00 : outi
sw_done:
        ld ix,(entry)
        ld c,(ix+3) : ld b,(ix+4) : ld e,(ix+5) : ld d,0
        call delay
        call count_to_vsync
        ld de,(rptr) : ld a,l : ld (de),a : inc de : ld a,h : ld (de),a : inc de : ld (rptr),de
        ld a,(trial) : inc a : ld (trial),a
        cp 65
        jp nz,sw_next
        ret

; BC passes (at least 1), E remainder
delay:
dl_pass: dec bc : ld a,b : or c : jr nz,dl_pass          ; 7 us a pass
        ld hl,sled_end : ld d,0 : or a : sbc hl,de
        jp (hl)
        nop : nop : nop : nop : nop : nop
sled_end:
        ret

; the row of E and . from the results (the control first)
show:
        ld hl,results : ld e,(hl) : inc hl : ld d,(hl) : inc hl   ; DE = control
        ld b,64
sh_l:   push bc : push de
        ld c,(hl) : inc hl : ld b,(hl) : inc hl
        push hl
        ld h,b : ld l,c
        or a : sbc hl,de
        ld a,'.'
        bit 7,h : jr nz,sh_p
        ld de,10 : or a : sbc hl,de
        jr c,sh_p
        ld a,'E'
sh_p:   call TXT_OUTPUT
        pop hl : pop de : pop bc
        djnz sh_l
        ld hl,crlf : jp print_str

; wait for the start of a VSYNC (PPI port B bit 0), then select R0
wait_vsync:
        di
        ld b,#f5
wv_out: in a,(c) : rra : jr c,wv_out
wv_in:  in a,(c) : rra : jr nc,wv_in
        ld bc,#bc00 : out (c),c
        ret

; HL = passes of a 10 us loop until the next VSYNC start
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

variant: db 0
trial:   db 0
entry:   dw 0
rptr:    dw 0
v1:      db #7f
v2:      db #3f
title:  db "R0ENLARGE (CRTC 0)",13,10
        db "E = R0=#7F was in time (line enlarged)",13,10,13,10,0
txt_a:  db "OUT (C),C",13,10,0
txt_b:  db "OUTI",13,10,0
crlf:   db 13,10,0
txt_end: db 13,10,"One character per us of delay.",13,10
        db "Photo this screen, then a key",13,10,0
tab:    ds 64*6
results: ds 65*2
prog_end:

        save "R0ENL.BIN", start, prog_end-start, AMSDOS, "r0enlarge.dsk"
