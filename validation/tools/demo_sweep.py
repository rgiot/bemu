#!/usr/bin/env python3
"""Snapshots of a demo at every screenshot of its CSL script (taken by CPCSyntaxError), then each snapshot
played in amsrust and in CPCSyntaxError and compared frame by frame (snap_compare.py).

  demo_sweep.py <demo name> [frames=8] [out_dir]        e.g. demo_sweep.py amazing

The worst moments (share of 4x8 blocks that differ, best of -2..+2 frames) are listed first; their
side-by-side images (ours above, CPCSE below) are in out_dir. A difference is a lead, not a verdict:
CPCSyntaxError is not ground truth, so look at the picture and ask AMSpiriT (same .sna as argument)."""
import os, re, subprocess, sys, tempfile
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
WORK = os.environ.get('SHAKER_WORK', os.path.expanduser('~/shaker_work'))
CPCSE = os.environ.get('CPCSE', os.path.join(WORK, 'cpcse', 'build', 'cpcse'))
name = sys.argv[1]; frames = sys.argv[2] if len(sys.argv) > 2 else '8'
out = sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp()
os.makedirs(out, exist_ok=True)
ddir = os.path.join(REPO, 'validation', 'demos', name)
src = open(os.path.join(ddir, name + '.csl')).read()
src = re.sub(r"disk_insert '([^/][^']*)'", lambda m: f"disk_insert '{os.path.join(ddir, m.group(1))}'", src)
src = re.sub(r"screenshot_name '([^']*)'\nscreenshot", r"snapshot_name '\1'\nsnapshot", src)
open(os.path.join(out, 'snaps.csl'), 'w').write(src)
subprocess.run([CPCSE, '--csl', os.path.join(out, 'snaps.csl'), '--out', os.path.join(out, 'sna'), '--crtc', '0', '--emu-name', 'CPCSE'],
               capture_output=True, cwd=os.path.dirname(CPCSE), stdin=subprocess.DEVNULL)
snas = sorted(f for f in os.listdir(os.path.join(out, 'sna')) if f.endswith('.sna'))
rows = []
for f in snas:
    d = os.path.join(out, f[:-4])
    r = subprocess.run([sys.executable, os.path.join(REPO, 'validation/tools/snap_compare.py'), os.path.join(out, 'sna', f), frames, '20000', d],
                       capture_output=True, text=True, env=dict(os.environ, CPCSE=CPCSE))
    m = re.search(r'mean ([0-9.]+)', r.stdout)
    rows.append((float(m.group(1)) if m else -1, f))
    print(f'{f:30s} {rows[-1][0]:6.1f}%', flush=True)
print('\nworst first:')
for v, f in sorted(rows, reverse=True)[:8]:
    print(f'{v:6.1f}%  {f}   ({out}/{f[:-4]}/worst_*.png)')
