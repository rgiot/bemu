import sys, glob, os, numpy as np
from cmplib import lo, ams, mk
S = sys.argv[1]
for f in sorted(glob.glob(S + '/ams_modesplit_*.png')):
    name = os.path.basename(f)[4:-4]
    A = mk(ams(f)); B = mk(lo(glob.glob(S + '/rs_' + name + '/*.png')[0]))
    r0, r1 = 44, 180
    best = None
    for dx in range(-6, 7):
        a = A[r0:r1, 40:700]; b = B[r0:r1, 40 + dx:700 + dx]
        m = (a != b).mean()
        if best is None or m < best[0]: best = (m, dx)
    m, dx = best
    a = A[r0:r1, 40:700]; b = B[r0:r1, 40 + dx:700 + dx]
    print('%-16s best dx=%2d mismatch=%.4f (%d px)' % (name, dx, m, int((a != b).sum())))
