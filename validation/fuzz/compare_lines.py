#!/usr/bin/env python3
"""Run a fuzz snapshot in amsrust and CPCSyntaxError and diff the CRTC lines.

  compare_lines.py <file.sna> [microseconds]

Needs $SHAKER_WORK/cpcse/build/cpcse (validation/tools/build_cpcse_oracle.sh),
and target/release/examples/fuzz_lines. Prints the first divergence of the
(C4, C9, VSYNC, length) sequences, or `same` (exit code 0).
"""
import os
import re
import subprocess
import sys
import tempfile

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
WORK = os.environ.get('SHAKER_WORK', os.path.expanduser('~/shaker_work'))
CPCSE = os.environ.get('CPCSE', os.path.join(WORK, 'cpcse', 'build', 'cpcse'))

def ours(sna, us):
    out = subprocess.run([os.path.join(REPO, 'target/release/examples/fuzz_lines'), sna, str(us)],
                         capture_output=True, text=True).stdout.split('\n')
    rows = [tuple(map(int, l.split())) for l in out if l.strip()]
    return rows[1:]            # the first line is the partial one the vsync rise cut

def cpcse(sna, us):
    csl = tempfile.NamedTemporaryFile('w', suffix='.csl', delete=False)
    csl.write(f"csl_version 1.0\ncrtc_select 0\nreset\nwait 500000\nsnapshot_load '{os.path.abspath(sna)}'\nwait {us}\n")
    csl.close()
    env = dict(os.environ, CPCSE_TRACE_LINE='1', CPCSE_TRACE_LINE_MAX='100000', CPCSE_TRACE_LINE_FROM='24')
    p = subprocess.run([CPCSE, '--csl', csl.name, '--out', tempfile.mkdtemp(), '--crtc', '0', '--emu-name', 'CPCSE'],
                       capture_output=True, text=True, env=env, cwd=os.path.dirname(CPCSE), stdin=subprocess.DEVNULL)
    rec = []
    for l in p.stderr.split('\n'):
        m = re.search(r'line f\d+\s+n=\s*\d+ C4=\s*(\d+) C9=\s*(\d+) .*?(VSYNC)?\s+h\d.*us=(\d+)', l)
        if m:
            rec.append((int(m.group(1)), int(m.group(2)), 1 if m.group(3) else 0, int(m.group(4))))
    # lengths from the character counter; align on the first VSYNC rise after the load
    rec = [r for r in rec if r[3] >= 500000]
    start = next((i for i in range(1, len(rec)) if rec[i][2] and not rec[i - 1][2]), None)
    if start is None:
        return []
    rec = rec[start:]
    rows = [(rec[i][0], rec[i][1], rec[i][2], rec[i + 1][3] - rec[i][3]) for i in range(len(rec) - 1)][1:]
    # CPCSE logs a row each time the 8 bit C0 wraps, even when no line ended: glue those back
    merged = []
    for r in rows:
        if merged and merged[-1][3] % 256 == 0 and merged[-1][:2] == r[:2]:
            merged[-1] = merged[-1][:3] + (merged[-1][3] + r[3],)
        else:
            merged.append(r)
    return merged

def main():
    sna = sys.argv[1]
    us = int(sys.argv[2]) if len(sys.argv) > 2 else 60000
    a, b = ours(sna, us), cpcse(sna, us)
    n = min(len(a), len(b))
    for i in range(n):
        if a[i] != b[i]:
            print(f'{sna}: line {i} differs (C4 C9 V len): ours {a[i]} cpcse {b[i]}')
            for k in range(max(0, i - 3), min(n, i + 4)):
                print('  ', k, 'ours', a[k], 'cpcse', b[k])
            return 1
    print(f'{sna}: same over {n} lines (ours {len(a)}, cpcse {len(b)})')
    return 0

sys.exit(main())
