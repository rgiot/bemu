import numpy as np
from PIL import Image
from slots import letter

def lo(p):
    return np.array(Image.open(p).convert('RGB')).astype(int)

def ams(p):
    return lo(p)[0::2]          # AMSpiriT doubles every line

def mk(img):
    """Letter map of a screenshot; the probe's two pen-0 inks (K, B) are merged."""
    h, w, _ = img.shape
    out = np.empty((h, w), dtype='<U1')
    for y in range(h):
        for x in range(w):
            c = letter(img[y, x]); out[y, x] = 'K' if c == 'B' else c
    return out
