#!/usr/bin/env python3
"""cpcse_ram.py <file.sna> <us> <addr hex> [len]: run a snapshot in CPCSyntaxError and print RAM bytes
(through a snapshot taken at the end: CSL has no memory dump)."""
import os, subprocess, sys, tempfile
WORK = os.environ.get('SHAKER_WORK', os.path.expanduser('~/shaker_work'))
CPCSE = os.environ.get('CPCSE', os.path.join(WORK, 'cpcse', 'build', 'cpcse'))
def cpcse_ram(sna, us, addr, n=1):
    out = tempfile.mkdtemp()
    csl = os.path.join(out, 's.csl')
    open(csl, 'w').write(f"csl_version 1.0\ncrtc_select 0\nreset\nwait 500000\nsnapshot_load '{os.path.abspath(sna)}'\nwait {us}\nsnapshot_name 'end'\nsnapshot\n")
    subprocess.run([CPCSE, '--csl', csl, '--out', out, '--crtc', '0', '--emu-name', 'CPCSE'], capture_output=True,
                   cwd=os.path.dirname(CPCSE), stdin=subprocess.DEVNULL)
    for root, _, files in os.walk(out):
        for f in files:
            if f.endswith('.sna'):
                d = open(os.path.join(root, f), 'rb').read()
                return d[256 + addr:256 + addr + n]
    return None
if __name__ == '__main__':
    r = cpcse_ram(sys.argv[1], int(sys.argv[2]), int(sys.argv[3], 16), int(sys.argv[4]) if len(sys.argv) > 4 else 1)
    print(' '.join(f'{b:02X}' for b in r) if r is not None else 'no snapshot')
