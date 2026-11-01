; Mode-splitting probe. Screen in mode OLD; on every even raster line the
; mode NEW is written at a different microsecond offset P (12 + line pair),
; around a short HSYNC (R2=46, R3=R3V), so the picture shows exactly how the
; Gate Array turns a mid-line mode write into pixels.
; Build: basm modesplit.asm --snapshot -o modesplit_<old>_<new>.sna  (defines OLD, NEW, R3V)

    ifndef OLD
OLD equ 1
    endif
    ifndef NEW
NEW equ 0
    endif
    ifndef R3V
R3V equ 2
    endif

    org &8000
    run START

START
    di
    ld sp,&7000
    ; pseudo-random pattern in the screen RAM
    ; screen: every byte is a flat mode-1 colour block; byte 0 of a character
    ; uses pen (k & 3), byte 1 pen ((k >> 2) & 3), k = character index & 15
    ld hl,&c000
    ld bc,&4000
.fill
    ld a,l
    srl a
    and 15              ; k
    bit 0,l
    jr z,.even
    srl a
    srl a
.even
    and 3
    ld e,a
    ld d,0
    push hl
    ld hl,BLOCKS
    add hl,de
    ld a,(hl)
    pop hl
    ld (hl),a
    inc hl
    dec bc
    ld a,b
    or c
    jr nz,.fill
    ; palette: 16 pens, well separated inks
    ld hl,INKS
    ld e,0
.pal
    ld bc,&7f00
    out (c),e
    ld a,(hl)
    out (c),a
    inc hl
    inc e
    ld a,e
    cp 16
    jr nz,.pal
    ld bc,&7f10
    out (c),c          ; border pen
    ld a,&58
    out (c),a          ; border ink: distinct from every pen
    ld bc,&7f00
    out (c),c          ; pen 0 stays selected: ink writes recolour pen 0
    ; CRTC
    ld hl,CRTC_TABLE
    ld bc,&bc00
.crtc
    out (c),c
    inc b
    ld a,(hl)
    out (c),a
    dec b
    inc hl
    inc c
    ld a,c
    cp 16
    jr nz,.crtc
    ld bc,&7f8c+OLD
    out (c),c

MAINLOOP
    ld b,&f5
.nov
    in a,(c)
    rra
    jr nc,.nov
    ifdef SSM
    defb &ed,&ff          ; SSM: screenshot request (AMSpiriT --ssm)
    endif
    ld b,&7f
    ld d,&8c+NEW
    ld e,&8c+OLD
    ld h,&44             ; ink B (marker colour, odd rows)
    ld l,&54              ; ink A (base colour of pen 0)
    ld a,28
.wl
    defs 60,0
    dec a
    jr nz,.wl
LINE = 0
    repeat 39
P = 12 + LINE
        ; even line: OLD, then NEW at ~P, then ink A at ~56
        defs 8,0
        out (c),e
        defs P-12,0
        out (c),d
        defs 52-P,0
        out (c),l
        defs 4,0
        ; odd line: OLD, then ink B at ~P (phase marker)
        defs 8,0
        out (c),e
        defs P-12,0
        out (c),h
        defs 64-P-4,0
LINE = LINE + 1
    rend
    jp MAINLOOP

BLOCKS
    defb &00,&f0,&0f,&ff       ; flat mode-1 byte for pens 0..3
INKS
    defb &54,&4b,&4c,&43,&5c,&4e,&47,&55,&4a,&4d,&46,&56,&4f,&57,&59,&5b
CRTC_TABLE
    defb 63, 63, 46, &80+R3V
    defb 38, 0, 25, 30
    defb 0, 7, 0, 0
    defb &30, 0, 0, 0
