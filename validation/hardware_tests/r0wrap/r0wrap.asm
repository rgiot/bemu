; R0WRAP - CRTC 0 hardware test: is the "last line of the frame" state of the
; CRTC reassessed when C0 wraps from 255 to 0 in the middle of a line?
;
; On the last line of the frame (C4=R4=38, C9=R9=7) the program lowers R0
; under C0 (C0 runs on to 255, wraps to 0 and counts up to R0), changes R4 to
; 0 after C0=1, and restores R0=63 while C0 is still beyond it. Then it counts
; how long the next VSYNC takes, in passes of a 10 usec loop:
;   - the frame restarts after the long line (state not reassessed at the
;     wrap): about 1500 passes (counted from the test line);
;   - the state is reassessed at the wrap with R4=0: C4 overflows up to 127
;     before coming back to R7, about 3 times longer (about 6000 passes).
; It does it 4 times, then once on an untouched frame (CONTROL, about 2000).
; amsrust 2026-11 (reassessing): TEST 6082 x4, CONTROL 1996; not reassessing: TEST 1525.
; Build: basm r0wrap.asm (writes R0WRAP.BIN on r0wrap.dsk)
; Run:   RUN"R0WRAP"

TXT_OUTPUT equ #bb5a
KM_WAIT_CHAR equ #bb06

        org #4000
start:
        ld hl,title : call print_str
        ld b,4
.tests:
        push bc
        call test_frame
        ld (result),hl
        ld hl,txt_test : call print_str
        ld hl,(result) : call print_dec
        ld hl,crlf : call print_str
        pop bc
        djnz .tests

        call control_frame
        ld (result),hl
        ld hl,txt_control : call print_str
        ld hl,(result) : call print_dec
        ld hl,txt_end : call print_str
        call KM_WAIT_CHAR
        ret

; wait for the start of a VSYNC (PPI port B bit 0), from outside of one
wait_vsync:
        ld b,#f5
.out:   in a,(c) : rra : jr c,.out
.in:    in a,(c) : rra : jr nc,.in
        ret

; HL = passes of a 10 usec loop until the next VSYNC start (through the end
; of the VSYNC running, if any)
count_to_vsync:
        ld hl,0
        ld b,#f5
.high:  inc hl              ; 2 us
        in a,(c)            ; 4 us
        rra                 ; 1 us
        jr c,.high          ; 3 us
.low:   inc hl              ; 2 us
        in a,(c)            ; 4 us
        rra                 ; 1 us
        jr nc,.low          ; 3 us
        ret

; an untouched frame: from a VSYNC start to the next one
control_frame:
        di
        call wait_vsync
        call count_to_vsync
        ei
        ret

test_frame:
        di
        call wait_vsync
        ; the VSYNC starts at C4=R7=30, C9=0. The last line of the frame
        ; (C4=38, C9=7) starts 71 lines (4544 us) later: get to its C0=12
        ld bc,DELAY
.wait:  dec bc              ; 2 us
        ld a,b              ; 1 us
        or c                ; 1 us
        jr nz,.wait         ; 3 us (7 us a pass)
        ; R0=3 while C0 is beyond it: C0 goes on to 255 and wraps
        ld bc,#bc00 : out (c),c
        ld bc,#bd03 : out (c),c
        ; R4=0 after C0=1 on the last line
        ld bc,#bc04 : out (c),c
        ld bc,#bd00 : out (c),c
        ; R0=63 again, C0 being past 63 by then: it still wraps at 255
        ld b,10
.w2:    djnz .w2
        ld bc,#bc00 : out (c),c
        ld bc,#bd3f : out (c),c
        ; wait for the long line to be over (C0 to 255, then 0..63), then R4=38
        ld b,80
.w3:    djnz .w3
        ld bc,#bc04 : out (c),c
        ld bc,#bd26 : out (c),c
        call count_to_vsync
        ei
        ret

; DELAY: the 7 us passes from the VSYNC detection to C0=12 of the last line
DELAY equ (4544 + 12 - 12) / 7

; print the zero-terminated string at HL
print_str:
        ld a,(hl) : or a : ret z
        call TXT_OUTPUT
        inc hl
        jr print_str

; print HL in decimal (5 digits)
print_dec:
        ld de,10000 : call .digit
        ld de,1000 : call .digit
        ld de,100 : call .digit
        ld de,10 : call .digit
        ld a,l : add '0' : jp TXT_OUTPUT
.digit:
        ld a,'0'-1
.sub:   inc a
        or a
        sbc hl,de
        jr nc,.sub
        add hl,de
        jp TXT_OUTPUT

result: dw 0
title:  db "R0WRAP (CRTC 0)",13,10
        db "Last line state after C0 255->0",13,10
        db "10 us passes to the next VSYNC:",13,10,13,10,0
txt_test: db "TEST    ",0
txt_control: db "CONTROL ",0
crlf:   db 13,10,0
txt_end: db 13,10,13,10,"TEST ~1500: not reassessed",13,10
        db "TEST ~6000: reassessed",13,10
        db "CONTROL ~2000: a normal frame",13,10,13,10
        db "Photo this screen, then a key",13,10,0
prog_end:

        save "R0WRAP.BIN", start, prog_end-start, AMSDOS, "r0wrap.dsk"
