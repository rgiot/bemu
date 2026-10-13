#!/usr/bin/env python3
"""Control flow of a demo, amsrust against CPCSyntaxError, from reset with the same CSL script.

  flow_compare.py <demo name> [from_us=0] [to_us] [min_jump=256]

Both emulators print the PC at every jump larger than min_jump; the two sequences are compared
(page of the target, interrupt entries below &0100 ignored) and the first divergence is reported with
the time offsets before it (a jump of the offset = a wait that lasts longer in one of them).
It needs `run_csl` and CPCSE (see build_cpcse_oracle.sh; the PC trace is the local patch of
cpcse's StepInstruction, CPCSE_TRACE_PC/_TO/_MIN)."""
import os, re, subprocess, sys
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
WORK = os.environ.get('SHAKER_WORK', os.path.expanduser('~/shaker_work'))
CPCSE = os.environ.get('CPCSE', os.path.join(WORK, 'cpcse', 'build', 'cpcse'))
name = sys.argv[1]
t0 = int(sys.argv[2]) if len(sys.argv) > 2 else 0
t1 = int(sys.argv[3]) if len(sys.argv) > 3 else 1 << 60
mj = int(sys.argv[4]) if len(sys.argv) > 4 else 256
ddir = os.path.join(REPO, 'validation', 'demos', name)
src = open(os.path.join(ddir, name + '.csl')).read()
src = re.sub(r"disk_insert '([^/][^']*)'", lambda m: f"disk_insert '{os.path.join(ddir, m.group(1))}'", src)
src = re.sub(r"screenshot_name '[^']*'\nscreenshot\n", "", src)
csl = f'/tmp/flow_{name}.csl'
open(csl, 'w').write(src)
def run(cmd, env, cwd=None):
    p = subprocess.run(cmd, capture_output=True, text=True, env=dict(os.environ, **env), cwd=cwd, stdin=subprocess.DEVNULL)
    rows = []
    for l in p.stderr.split('\n'):
        if l.startswith('PC '):
            _, t, pc = l.split()[:3]
            rows.append((int(t), int(pc, 16)))
    return rows
o = run([os.path.join(REPO, 'target/release/examples/run_csl'), csl, '/tmp/flow_out'],
        {'AMSRUST_TRACE_PC': str(t0), 'AMSRUST_TRACE_PC_TO': str(t1), 'AMSRUST_TRACE_PC_MIN': str(mj)})
c = run([CPCSE, '--csl', csl, '--out', '/tmp/flow_cpcse', '--crtc', '0', '--emu-name', 'CPCSE'],
        {'CPCSE_TRACE_PC': str(t0), 'CPCSE_TRACE_PC_TO': str(t1), 'CPCSE_TRACE_PC_MIN': str(mj), 'CPCSE_TRACE_PC_BUDGET': '50000000'},
        cwd=os.path.dirname(CPCSE))
def pages(rows):
    out = []; last = None
    for t, pc in rows:
        if pc < 0x100: continue
        pg = pc >> 8
        if pg != last: out.append((t, pg, pc)); last = pg
    return out
O, C = pages(o), pages(c)
print(f'{len(o)} / {len(c)} jumps, {len(O)} / {len(C)} page changes')
prev = None
for i, ((to, po, pco), (tc, pc_, pcc)) in enumerate(zip(O, C)):
    if abs(pco - pcc) > 8:
        print(f'first divergence at page change {i}: amsrust {to} us -> {pco:04X}   CPCSE {tc} us -> {pcc:04X}')
        print(' amsrust:', [(t, f'{p:04X}') for t, _, p in O[max(0, i - 4):i + 6]])
        print(' CPCSE  :', [(t, f'{p:04X}') for t, _, p in C[max(0, i - 4):i + 6]])
        break
    d = tc - to
    if prev is not None and abs(d - prev) > 5000:
        print(f'time offset moves by {d - prev} us at page change {i} ({to} us, page {po:02X})')
    prev = d
else:
    print('same page sequence over', min(len(O), len(C)), 'changes')
