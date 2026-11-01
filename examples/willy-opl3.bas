10 REM Willy's OPL3LPT (&FEBC): the AdLib tutorial's tone, 440 Hz on channel 1
20 FOR i=1 TO 9:READ r,v:OUT &FEBC,r:OUT &FEBD,v:NEXT
30 PRINT "OPL3: 440 Hz, a key to stop":CALL &BB18
40 OUT &FEBC,&B0:OUT &FEBD,&11:PRINT "stopped"
50 DATA &20,&01,&40,&10,&60,&F0,&80,&77,&A0,&98,&23,&01,&63,&F0,&83,&77,&B0,&31
