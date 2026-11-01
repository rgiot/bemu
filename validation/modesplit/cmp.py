import sys, numpy as np
from PIL import Image

def load_ams(path):
    a = np.array(Image.open(path).convert('RGB')).astype(int)
    # AMSpiriT doubles each scan line vertically
    rows = [a[y] for y in range(0, a.shape[0], 2)]
    return np.array(rows)

def load_rs(path):
    return np.array(Image.open(path).convert('RGB')).astype(int)

def edges(img):
    e = np.zeros(img.shape[:2], dtype=np.uint8)
    e[:, 1:] = (np.abs(img[:, 1:] - img[:, :-1]).sum(axis=2) > 0)
    return e

def best_align(ea, eb, maxdx=300, maxdy=100, r0=60, r1=110, c0=100, c1=300):
    A = ea[r0:r1, c0:c1]
    best = None
    for dy in range(-maxdy, maxdy + 1):
        if r0 + dy < 0 or r1 + dy > eb.shape[0]: continue
        for dx in range(-maxdx, maxdx + 1):
            if c0 + dx < 0 or c1 + dx > eb.shape[1]: continue
            B = eb[r0 + dy:r1 + dy, c0 + dx:c1 + dx]
            sc = float((A != B).mean())
            if best is None or sc < best[0]: best = (sc, dx, dy)
    return best

if __name__ == '__main__':
    A = load_ams(sys.argv[1]); B = load_rs(sys.argv[2])
    print('ams', A.shape, 'rs', B.shape)
    ea, eb = edges(A), edges(B)
    print('best align (score,dx,dy) mapping ams(y,x) -> rs(y+dy, x+dx):', best_align(ea, eb))

def rowdiff(A, B, dx, dy, c0=0, c1=700):
    ea, eb = edges(A), edges(B)
    out = []
    for y in range(A.shape[0]):
        yb = y + dy
        if yb < 0 or yb >= B.shape[0]: continue
        xs = [x for x in range(max(c0, 1), c1) if 0 <= x + dx < B.shape[1] and ea[y, x] != eb[yb, x + dx]]
        if xs: out.append((y, len(xs), xs[0], xs[-1]))
    return out
