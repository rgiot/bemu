; DRIVETIME - how long AMSDOS takes to load, measured by the CPC itself with the firmware's clock
; (KL TIME PLEASE: 1/300 s). Run it on a real CPC and in emulators: the numbers are comparable.
;
;   BIG     a 36K file (the motor starts first: its spin-up is included)
;   CAT     the catalogue
;   FAR     a 1K file near the last track (the head has to travel)
;   SMALL   8 files of 1K, one after the other
;
; The results stay in memory at RESULTS (4 words, 1/300 s each) for debuggers.
; Build: basm drivetime.asm && python3 make_disc.py (writes drivetime.dsk). Run: RUN"DTIME"

TXT_OUTPUT   equ #bb5a
KM_WAIT_CHAR equ #bb06
CAS_IN_OPEN  equ #bc77
CAS_IN_CLOSE equ #bc7a
CAS_IN_DIRECT equ #bc83
CAS_CATALOG  equ #bc9b
KL_TIME_PLEASE equ #bd0d
SCR_SET_MODE equ #bc0e
KL_INIT_BACK equ #bcce

BUFFER  equ #9800           ; 2K for AMSDOS (under the program, above the files)
LOAD_AT equ #0400           ; where the files are loaded

        org #a000
start:
        ; RUN"DTIME" leaves only the firmware: bring AMSDOS (ROM 7) back
        ld c,7 : ld de,#0040 : ld hl,#abff : call KL_INIT_BACK
        ld a,1 : call SCR_SET_MODE

        call clock_start
        ld hl,name_big : ld b,name_big_end-name_big : call load_file
        call clock_stop : ld (RESULTS+0),hl

        call clock_start
        ld de,BUFFER : call CAS_CATALOG
        call clock_stop : ld (RESULTS+2),hl

        call clock_start
        ld hl,name_far : ld b,name_far_end-name_far : call load_file
        call clock_stop : ld (RESULTS+4),hl

        call clock_start
        ld a,'0'
.small: ld (name_small+5),a
        push af
        ld hl,name_small : ld b,name_small_end-name_small : call load_file
        pop af
        inc a
        cp '8'
        jr nz,.small
        call clock_stop : ld (RESULTS+6),hl

        ld hl,title : call print_str
        ld hl,txt_big : call print_str : ld hl,(RESULTS+0) : call print_line
        ld hl,txt_cat : call print_str : ld hl,(RESULTS+2) : call print_line
        ld hl,txt_far : call print_str : ld hl,(RESULTS+4) : call print_line
        ld hl,txt_small : call print_str : ld hl,(RESULTS+6) : call print_line
        ld hl,txt_end : call print_str
        call KM_WAIT_CHAR
        ret

; load the file named at HL (B characters) at LOAD_AT
load_file:
        ld de,BUFFER
        call CAS_IN_OPEN
        ret nc
        ld hl,LOAD_AT
        call CAS_IN_DIRECT
        jp CAS_IN_CLOSE

clock_start:
        call KL_TIME_PLEASE
        ld (t0),hl
        ret

; HL = 1/300 s since clock_start
clock_stop:
        call KL_TIME_PLEASE
        ld de,(t0)
        or a
        sbc hl,de
        ret

print_str:
        ld a,(hl) : or a : ret z
        call TXT_OUTPUT
        inc hl
        jr print_str

; HL in decimal, then a new line
print_line:
        ld de,10000 : call .digit
        ld de,1000 : call .digit
        ld de,100 : call .digit
        ld de,10 : call .digit
        ld a,l : add '0' : call TXT_OUTPUT
        ld hl,crlf : jp print_str
.digit:
        ld a,'0'-1
.sub:   inc a
        or a
        sbc hl,de
        jr nc,.sub
        add hl,de
        jp TXT_OUTPUT

t0:     dw 0
name_big:   db "BIG.BIN"
name_big_end:
name_far:   db "FAR.BIN"
name_far_end:
name_small: db "SMALL0.BIN"
name_small_end:
title:  db 12,"DRIVETIME (1/300 s)",13,10,13,10,0
txt_big:   db "BIG 36K  ",0
txt_cat:   db "CAT      ",0
txt_far:   db "FAR 1K   ",0
txt_small: db "SMALL 8K ",0
crlf:   db 13,10,0
txt_end: db 13,10,"Photo this screen, then a key",13,10,0
RESULTS = #a600
prog_end:

        save "dtime.raw", start, prog_end-start
