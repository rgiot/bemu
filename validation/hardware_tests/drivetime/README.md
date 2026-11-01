# DRIVETIME: how long AMSDOS takes to load

The program times AMSDOS itself with the firmware's clock (`KL TIME PLEASE`, 1/300 s), so the
numbers mean the same on a real CPC and in emulators:

| line | what is timed |
|---|---|
| `BIG 36K` | loading a 36K file, the drive motor starting first (spin-up included) |
| `CAT` | the catalogue |
| `FAR 1K` | a 1K file on track 34 (the head travels from track 0 and back) |
| `SMALL 8K` | 8 files of 1K, one after the other (each reads the directory on track 0 first) |

The results also stay in memory at &A600 (4 words) for debuggers.

| (1/300 s) | BIG 36K | CAT | FAR 1K | SMALL 8K |
|---|---|---|---|---|
| amsrust (2026-11-01) | 1215 | 729 | 201 | 5212 |
| amsrust, 3.5" drive (`--drive-a 3.5`) | 1215 | 729 | 201 | 5212 |
| ACE-DL (2026-11, default 3" drive) | 1215 | 727 | 203 | 5219 |
| ACE-DL, Matsushita EME-150 3" (`DRIVEA=0`) | 1209 | 728 | 202 | 5221 |
| ACE-DL, Generic Alps 3"5 (`DRIVEA=1`) | 1222 | 722 | 201 | 5212 |
| ACE-DL, Gotek (`DRIVEA=2`) | 1216 | 730 | 200 | 5224 |
| real CPC | ? | ? | ? | ? |

The drive model hardly matters here: AMSDOS waits a fixed time after starting the motor, the FDC
sets the step rate, and 297 or 300 rpm is 1%. Two ACE runs of the same drive differ by as much
as the models do (where the disc is in its turn when a read starts).

## On a real CPC (6128, or 464/664 with a disc drive)

1. Put `drivetime.dsk` on the disc emulator or a 3" disc, in drive A.
2. Switch on, type `RUN"DTIME"` and RETURN. Do not touch the drive while it runs (about 25 s).
3. Photograph the 4 numbers. Do it 3 times (switching off between runs) and note the drive
   (internal 3", FD-1, Gotek... ) and the machine.

## Rebuilding

    basm drivetime.asm      # dtime.raw
    python3 make_disc.py    # drivetime.dsk (DATA format, the files one after the other)
    cargo run --release --example run_csl -- validation/hardware_tests/drivetime/drivetime.csl /tmp/out

In ACE-DL, its web API (`-enable_webapi -web_port 8899`, raw JSON over TCP) reads the results:
`printf '{"cmd":"readMemory","memType":"ram","address":42496,"size":8}' | nc 127.0.0.1 8899`
(the key is `size`; `length` gives "Invalid address/size"). The drive model is `DRIVEA=` in a
file given by `-configFile`.
