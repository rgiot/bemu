import sys, numpy as np
from collections import Counter
from cmp import load_ams, load_rs

def rowinfo(img, y, x0, x1):
    row = img[y, x0:x1]
    cols = Counter(map(tuple, row))
    return row, cols

def analyze(img, name, x0, x1, y0, y1):
    # palette guess: the border colour is the colour at far-left blank gap region; find the 6 dominant colours
    cnt = Counter()
    for y in range(y0, y1):
        cnt.update(map(tuple, img[y, x0:x1]))
    print(name, 'dominant colours:', cnt.most_common(8))
    return cnt

if __name__ == '__main__':
    A = load_ams(sys.argv[1]); B = load_rs(sys.argv[2])
    analyze(A, 'AMS', 0, 768, 40, 120)
    analyze(B, 'RS', 96, 832, 40, 120)
