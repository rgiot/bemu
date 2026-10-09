import sys, glob, numpy as np
from cmplib import lo, ams, mk
S, name, dx = sys.argv[1], sys.argv[2], int(sys.argv[3])
A = mk(ams(S + '/ams_' + name + '.png')); B = mk(lo(glob.glob(S + '/rs_' + name + '/*.png')[0]))
for y in range(44, 180):
    a = A[y, 40:700]; b = B[y, 40 + dx:700 + dx]
    idx = np.nonzero(a != b)[0]
    if len(idx):
        runs = []; st = pv = idx[0]
        for i in idx[1:]:
            if i != pv + 1: runs.append((st, pv)); st = i
            pv = i
        runs.append((st, pv))
        print(y, ' | '.join('x%d-%d %s>%s' % (s + 40, e + 40, ''.join(a[s:e+1])[:12], ''.join(b[s:e+1])[:12]) for s, e in runs[:4]))
