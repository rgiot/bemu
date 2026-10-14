; Assembled with basm (cpclib) into basm_breakpoints.sna:
;   basm basm_breakpoints.asm --snapshot -o basm_breakpoints.sna
; Read by snapshot::breakpoint_test::breakpoints_of_a_basm_snapshot.
	org #4000
	run start
start
	breakpoint
	ld bc,#7f10
	ld a,#54
loop
	out (c),c
	out (c),a
	ld (#c000),a
	inc a
	and #5f
	or #40
	jr loop
	breakpoint #c000, type=mem, access=write, runmode=watch, size=16, name="screen"
	breakpoint #7f00, type=io, access=write, runmode=stop, mask=#ff00, value=#40, valmask=#c0, condition="A==#55", name="ink"
