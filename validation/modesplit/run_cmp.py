import sys
from cmp import *
A = load_ams(sys.argv[1]); B = load_rs(sys.argv[2])
ea, eb = edges(A), edges(B)
sc, dx, dy = best_align(ea, eb)
print('align', sc, dx, dy)
for y, n, x0, x1 in rowdiff(A, B, dx, dy):
    print(f'ams row {y:3d}: {n:4d} edge mismatches x {x0}..{x1}')
