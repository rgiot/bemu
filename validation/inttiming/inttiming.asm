; Interrupt-timing probe. EI, HALT; after the interrupt returns, pen 0 is
; recoloured white for 8 us on each of 24 consecutive 64 us lines, the mark
; starting 1 us later on every line: the screen shows where, in raster
; position, the code runs after the interrupt (compare emulators: shift of the
; diagonal = difference in interrupt latency/response).
; Build: basm inttiming.asm --snapshot -o inttiming.sna

    org &8000
    run START

START
    di
    ld sp,&7000
    ld a,&fb          ; EI
    ld (&38),a
    ld a,&c9          ; RET
    ld (&39),a
    ld bc,&7f8d       ; mode 1, ROMs off
    out (c),c
    ld bc,&7f00       ; select pen 0
    out (c),c
    ld h,&4b          ; white ink
    ld l,&54          ; black ink
    ld b,&7f
    im 1
    ei
MAINLOOP
    halt
LINE = 0
    repeat 24
        defs 4,0
        defs LINE,0
        out (c),h
        out (c),l
        defs 64-4-LINE-8,0
LINE = LINE + 1
    rend
    jp MAINLOOP
