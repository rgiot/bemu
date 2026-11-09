; POLLPHASE - where does a line of code land against the CRTC after a VSYNC poll?
;
; Every frame: wait for the VSYNC with the usual PPI poll (IN A,(C) / RRA / JR), wait a fixed time,
; then three bands in the top border, 8 lines apart: the border turns white with OUT (C),C, black
; again 10 us later; the same with OUT (n),A and with OUTI. A band's x position on the screen is the
; time since its line's start (16 pixels of the screenshot's 768 a microsecond): the poll's exit
; phase and the instruction's effect together. The same disc on two emulators must give the same
; bands; the differences between the bands of one emulator tell about the instructions, the
; offset of the three together about the poll.
; Build: basm pollphase.asm (writes POLLPH.BIN on pollphase.dsk)   Run: RUN"POLLPH"

LINES equ 36                ; lines from the poll to the first band's line
GAP equ 512-60              ; between the bands: about 8 lines

        org #4000
start:
        di
        ld bc,#7f10 : out (c),c           ; select the border pen
frame:
        ld b,#f5
w1:     in a,(c) : rra : jr c,w1          ; wait for the VSYNC to be over
w2:     in a,(c) : rra : jr nc,w2         ; and for the next one
        ld bc,LINES*64/7 : ld e,(LINES*64) % 7
        call delay
        ; band 1: OUT (C),C
        ld bc,#7f4b : out (c),c           ; white
        nop : nop : nop : nop : nop : nop : nop : nop : nop : nop
        ld bc,#7f54 : out (c),c           ; black
        ld bc,GAP/7 : ld e,GAP % 7 : call delay
        ; band 2: OUT (n),A
        ld a,#4b : out (#7f),a
        nop : nop : nop : nop : nop : nop : nop : nop : nop : nop
        ld a,#54 : out (#7f),a
        ld bc,GAP/7 : ld e,GAP % 7 : call delay
        ; band 3: OUTI
        ld hl,white : ld bc,#8000
        outi
        nop : nop : nop : nop : nop : nop : nop : nop : nop : nop
        ld hl,black : ld bc,#8000
        outi
        jp frame

white:  db #4b
black:  db #54

; BC passes of 7 us (at least 1), E the remainder (0 to 6)
delay:
dl_pass: dec bc : ld a,b : or c : jr nz,dl_pass
        ld hl,sled_end : ld d,0 : or a : sbc hl,de
        jp (hl)
        nop : nop : nop : nop : nop : nop
sled_end:
        ret
prog_end:
        save "POLLPH.BIN", start, prog_end-start, AMSDOS, "pollphase.dsk"
