#!/usr/bin/env python3
"""Run the same .sna in amsrust and in CPCSyntaxError and compare the pictures frame by frame.

  snap_compare.py <file.sna> [frames=40] [step_us=20000] [out_dir]

Both start from the same Z80/RAM/register state, so the pictures of a demo can be compared
at the same moment (a snapshot of a demo part is taken with CPCSyntaxError's CSL `snapshot`).
Prints per frame the share of 4x8 blocks that differ (ours against CPCSE, shifted by -2..2 frames,
best kept) and writes side-by-side images of the worst frames. Needs $CPCSE (see build_cpcse_oracle.sh)."""
import os, subprocess, sys, tempfile
import numpy as np
from PIL import Image
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
WORK = os.environ.get('SHAKER_WORK', os.path.expanduser('~/shaker_work'))
CPCSE = os.environ.get('CPCSE', os.path.join(WORK, 'cpcse', 'build', 'cpcse'))
sna = os.path.abspath(sys.argv[1]); n = int(sys.argv[2]) if len(sys.argv) > 2 else 40
step = int(sys.argv[3]) if len(sys.argv) > 3 else 20000
out = sys.argv[4] if len(sys.argv) > 4 else tempfile.mkdtemp()
os.makedirs(out, exist_ok=True)
shots = ''.join(f"wait {step}\nscreenshot_name 'f{i:03d}'\nscreenshot\n" for i in range(n))
open(os.path.join(out, 'ours.csl'), 'w').write("csl_version 1.0\n" + shots)
open(os.path.join(out, 'cpcse.csl'), 'w').write(f"csl_version 1.0\ncrtc_select 0\nreset\nwait 500000\nsnapshot_load '{sna}'\n" + shots)
subprocess.run([os.path.join(REPO, 'target/release/examples/probe_sna_script'), sna, os.path.join(out, 'ours.csl'), os.path.join(out, 'o'), '0'], capture_output=True)
subprocess.run([CPCSE, '--csl', os.path.join(out, 'cpcse.csl'), '--out', os.path.join(out, 'c'), '--crtc', '0', '--emu-name', 'CPCSE'],
               capture_output=True, cwd=os.path.dirname(CPCSE), stdin=subprocess.DEVNULL)
def load(p):
    return np.array(Image.open(p).convert('RGB').resize((768, 272), Image.NEAREST)).astype(int)
def blocks(a, b):
    d = np.abs(a - b).sum(2)
    d = np.where(d > 24, d, 0)[:272 // 8 * 8, :768 // 4 * 4]   # our blacks are 0/1/0 percent
    d = d.reshape(272 // 8, 8, 768 // 4, 4).sum((1, 3))
    return float((d > 90).mean() * 100)
O = [load(os.path.join(out, 'o', f'f{i:03d}.png')) for i in range(n)]
C = [load(os.path.join(out, 'c', f'f{i:03d}.bmp')) for i in range(n)]
res = []
for i in range(2, n - 2):
    best = min((blocks(O[i], C[i + s]), s) for s in range(-2, 3))
    res.append((i, best))
    print(i, f'{best[0]:.1f}% shift {best[1]}')
print('mean', sum(b[0] for _, b in res) / len(res))
worst = sorted(res, key=lambda r: -r[1][0])[:4]
for i, (v, s) in worst:
    W = Image.new('RGB', (768, 544)); W.paste(Image.fromarray(O[i].astype('uint8')), (0, 0)); W.paste(Image.fromarray(C[i + s].astype('uint8')), (0, 272))
    W.save(os.path.join(out, f'worst_{i:03d}.png'))
