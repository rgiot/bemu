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

