10 REM Willy's S2P (&FEAC): its detection, then a scale on a piano; then the OPL3's tone
20 a=INP(&FEAD):OUT &FEAD,&3F:b=INP(&FEAD):c=INP(&FEAC)
30 PRINT "S2P: &";HEX$(a,2);" &";HEX$(b,2);" &";HEX$(c,2);" (&80 &C0 &FE expected)"
40 FOR i=1 TO 8:READ n:OUT &FEAC,&90:OUT &FEAC,n:OUT &FEAC,100:FOR t=1 TO 300:NEXT:OUT &FEAC,&80:OUT &FEAC,n:OUT &FEAC,0:NEXT
50 DATA 60,62,64,65,67,69,71,72
60 REM Willy's OPL3LPT (&FEBC): the AdLib tutorial's tone, 440 Hz on channel 1
70 FOR i=1 TO 9:READ r,v:OUT &FEBC,r:OUT &FEBD,v:NEXT
80 PRINT "OPL3: 440 Hz, a key to stop":CALL &BB18
90 OUT &FEBC,&B0:OUT &FEBD,&11:PRINT "stopped"
100 DATA &20,&01,&40,&10,&60,&F0,&80,&77,&A0,&98,&23,&01,&63,&F0,&83,&77,&B0,&31
