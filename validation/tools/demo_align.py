#!/usr/bin/env python3
"""Play a demo from reset in amsrust and in CPCSyntaxError with the same screenshot cadence, then match each
amsrust picture with the closest CPCSyntaxError one (within +-window shots: disc loading time differs) and
list the pictures that have no good match. CPCSyntaxError is not ground truth: look at the listed pairs.

  demo_align.py <demo name> [step_s=0.5] [count=120] [window=8] [out_dir]"""
import os, re, subprocess, sys, tempfile
import numpy as np
from PIL import Image
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
WORK = os.environ.get('SHAKER_WORK', os.path.expanduser('~/shaker_work'))
CPCSE = os.environ.get('CPCSE', os.path.join(WORK, 'cpcse', 'build', 'cpcse'))
name = sys.argv[1]
step = float(sys.argv[2]) if len(sys.argv) > 2 else 0.5
count = int(sys.argv[3]) if len(sys.argv) > 3 else 120
window = int(sys.argv[4]) if len(sys.argv) > 4 else 8
out = sys.argv[5] if len(sys.argv) > 5 else tempfile.mkdtemp()
os.makedirs(out, exist_ok=True)
ddir = os.path.join(REPO, 'validation', 'demos', name)
src = open(os.path.join(ddir, name + '.csl')).read()
head = src.split('wait', 2)  # keep everything up to the first screenshot
lines = src.split('\n')
keep = []
for l in lines:
    if l.startswith('screenshot') or l.startswith('wait ') and keep and keep[-1].startswith('wait') is False and False:
        pass
# header: everything before the first screenshot_name, minus the waits that follow the run command
idx = next(i for i, l in enumerate(lines) if l.startswith('screenshot_name'))
header = '\n'.join(lines[:idx - 1]) + '\n'      # drops the `wait N` right before the first screenshot
header = re.sub(r"disk_insert '([^/][^']*)'", lambda m: f"disk_insert '{os.path.join(ddir, m.group(1))}'", header)
tail = '\n'.join(l for l in lines[idx - 1:] if l.startswith('key_output'))   # demos that need keys: handled by hand
shots = ''.join(f"wait {int(step * 1e6)}\nscreenshot_name 's{i:04d}'\nscreenshot\n" for i in range(count))
open(os.path.join(out, 'run.csl'), 'w').write(header + shots)
subprocess.run([os.path.join(REPO, 'target/release/examples/run_csl'), os.path.join(out, 'run.csl'), os.path.join(out, 'o')], capture_output=True)
subprocess.run([CPCSE, '--csl', os.path.join(out, 'run.csl'), '--out', os.path.join(out, 'c'), '--crtc', '0', '--emu-name', 'CPCSE'],
               capture_output=True, cwd=os.path.dirname(CPCSE), stdin=subprocess.DEVNULL)
def desc(p):
    a = np.array(Image.open(p).convert('RGB').resize((192, 68), Image.BILINEAR)).astype(float)
    return a
def find(d, i, ext):
    for e in ext:
        p = os.path.join(d, f's{i:04d}.{e}')
        if os.path.exists(p): return p
O = [desc(find(os.path.join(out, 'o'), i, ['png'])) for i in range(count)]
C = [desc(find(os.path.join(out, 'c'), i, ['bmp', 'png'])) for i in range(count)]
bad = []
for i in range(count):
    best = min((np.abs(O[i] - C[j]).mean(), j - i) for j in range(max(0, i - window), min(count, i + window + 1)))
    flag = '  <==' if best[0] > 6 else ''
    print(f'{i:4d} ({i * step:6.1f}s) diff {best[0]:6.2f} shift {best[1]:+d}{flag}')
    if best[0] > 6: bad.append((best[0], i, best[1]))
for v, i, s in sorted(bad, reverse=True)[:10]:
    a = Image.open(find(os.path.join(out, 'o'), i, ['png'])).convert('RGB').resize((768, 272))
    b = Image.open(find(os.path.join(out, 'c'), i + s, ['bmp', 'png'])).convert('RGB').resize((768, 272))
    W = Image.new('RGB', (768, 544)); W.paste(a, (0, 0)); W.paste(b, (0, 272)); W.save(os.path.join(out, f'pair_{i:04d}.png'))
print('pairs:', os.path.join(out, 'pair_*.png'))
