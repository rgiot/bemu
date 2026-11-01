# Demos used as emulator checks (always CRTC 0)

Each folder holds the disc and a CSL script that boots it on a CRTC 0 machine and
takes a screenshot every 5 seconds:

    cargo run --release --example run_csl -- validation/demos/<name>/<name>.csl /tmp/out

| folder | demo | run command |
|---|---|---|
| dsc4 | Logon System, DSC4 | `RUN"-DSC4"` |
| dtc | DTC (Arkos), prints "Crtc 0 detected / Nice one" with the interlace detection | `RUN"-DTC"` |
| borderline | Pulpo Corrosivo, Borderline (BCP24) | `RUN"BORDER"` |
| checkmate | Pulpo Corrosivo, Checkmate (Revision 23, 4K) | `RUN"CHECK"` |
| ghostnop | Pulpo Corrosivo and Futurs', Ghost NOP (Revision 24) | `RUN"GHOSTNOP"` |
| oc2 | Vanity, Onescreen Colonies #2 | `RUN"-OSC2"` |
| oc4 | Vanity, Onescreen Colonies #4 | `RUN"-OSC4"` |
| foursome | Pulpo Corrosivo, Foursome (Benediction Coding Party 2022, scene.org) | `RUN"FOURSOME"` |
| ovfprev | Overflow (Logon System), 5 previews 1992 (genesis8.free.fr). The loader asks `PREVIEW (1/5) ?`: type the preview number, then a key; ESC leaves to BASIC, so the script resets between previews | `RUN"-PREVIEW"`, `1`..`5`, space |
| stillscrolling | Still Scrolling (Lost, Found and More, scene.org). `CREDITS.BAS` waits for a key, then runs it. It does not run on CRTC 1/2 | `RUN"CREDITS"`, space |
| wcw | Benediction, What a Cool Weekend (2000, scene.org). Press space for the second part (the waving screens: smooth sine on AMSpiriT, jagged on amsrust) | `RUN"-WACWE!"`, space |

`../../examples/probe_pc_sample.rs` lists what the Z80 is doing when a demo waits (the ovfprev prompt is a BASIC INPUT).
| amazing | Logon System, The Amazing Megademo (rev 2, 2021; cpcrulez.fr/f/9ta). All CRTCs: it prints `CRTC 0 detected`, then a one-line rupture (RVI/RVLL) intro with a QR code page | `RUN"AMAZING"` |
| camenbert4 | Arkos, Camenbert 4 (C-Meeting 2004, scene.org). Loader says `CRTC n DETECTED` on a real machine; amsrust (CRTC 1) says `EMULATOR DETECTED : CRTC 1` and the plasma is green where AMSpiriT draws a rainbow: a good test of the limits | `RUN"-CAMENB4"` |
| stillone | iMPACT, Still The One (2023). Needs CRTC 1 and the 512K DK'tronics RAM (`memory_exp 4`, CSL v1.1) | `RUN"STO.BAS"` |
| sp97 | Benediction, System Party 97. Menu: 1 = Eliot's part (an interlude that waits for key `1`, then a maze), 2 = Rainbird's part (the waving rasters; `sp97.csl` starts the menu, the screenshots after are the menu itself: press `2`). The file name has a quote: `a$="-SP"+CHR$(39)+"97":RUN a$` | see script |
| rhino | Mad-Ram, Rhino-Raster (1993): a mode 2 picture in 3 colours, the silhouette made by `OUT` to the pen's ink timed to the microsecond after an `EI`+`HALT`. Needs a key (space) after the title. `../tools/rhino_compare.py` counts the pixels that differ from AMSpiriT's picture (`amspirit_1.13.4_crtc1.png`). Found that a palette write reaches the pixels already in the GA's pipeline, the video byte being fetched about 1 us before it is displayed: 20 pixels back on the GA 40010 (picture identical to ACE-DL's, `ace_dl.png`), 21 on the 40007/8 (identical to AMSpiriT Lite's, with its tiny black pixel); `cargo test --release --test rhino_raster -- --ignored`. The disc is `validation/rhino_mdr.dsk` | `RUN"-RHINO"`, space |
| skoh | S&KOH (its scroller says "achieved the 20th of July 1991"; LOGON bands in part 2). Part 1: bouncing logos between rainbow bars on 16 us lines (R0=15): an HSYNC every 16 us cuts the C-VSYNC into 12 us pulses, the monitor still has to lock (its sync separator integrates). Space loads the Overflow part (`YOUR CRTC:0 WAIT WHILE LOADING`), whose pictures scroll in an R.V.I. window under a big scroller (torn letters on AMSpiriT too) | `RUN"S&KOH"`, space |
| batman | Batman Forever (Batman Group, scene.org), the zip as published: the 3" set (disk 1 side A/B, disk 2 side A) and the one disk version, which needs a double-sided 3.5" drive (on a 3" one it says `WRONG DISK FOR YOUR CONFIGURATION`). `batman_3.csl` plays disk 1 and inserts side B when asked; `batman_35.csl` plays the one disk version: `run_csl validation/demos/batman/batman_35.csl /tmp/out --drive-a 3.5`. The scripts take the discs from the zip (`batman_forever.zip#Batman Forever (Disk 1 side A).dsk`) | `RUN"DISC"`, space |
| scrollfactory | Impact, Scroll Factory (2018, astimpact.weebly.com), the Complete version: scrolls of all kinds and the "Vanity HBL Killer" (HBL position). 128K, all CRTCs | `RUN"SCROLLF"` |
