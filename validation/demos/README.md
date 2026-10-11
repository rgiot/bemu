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
