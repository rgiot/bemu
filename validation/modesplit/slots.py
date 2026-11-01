import sys, numpy as np
from cmp import load_ams, load_rs
L = {(0,2,1):'K',(255,243,249):'W',(255,242,248):'W',(243,5,6):'R',(242,4,6):'R',(243,243,109):'Y',(242,242,109):'Y',(0,2,107):'B',(0,2,106):'B',(105,2,104):'P'}
def letter(c):
    t=tuple(int(v) for v in c)
    for k,v in L.items():
        if all(abs(t[i]-k[i])<=3 for i in range(3)): return v
    return '?'
def border_x(row):
    # first x where a blue run >= 14 px starts, scanning from x=20
    n=len(row); x=20
    while x<n-16:
        if letter(row[x])=='P' and all(letter(row[x+i])=='P' for i in range(14)): return x
        x+=1
    return None
def slotstr(img,y,x0,x1):
    row=img[y]
    bx=border_x(row[x0:x1]); 
    if bx is None: return None,''
    bx+=x0
    start=bx%8
    s=''.join(letter(row[x+4]) for x in range(x0+((start-x0)%8),x1-4,8))
    return bx,s
if __name__=='__main__':
    A=load_ams(sys.argv[1]); B=load_rs(sys.argv[2]); ay0,by0=int(sys.argv[3]),int(sys.argv[4]); n=int(sys.argv[5])
    for k in range(n):
        bx,s=slotstr(A,ay0+k,0,768); print('AMS %3d bx=%s %s'%(ay0+k,bx,s[:70]))
    for k in range(n):
        bx,s=slotstr(B,by0+k,0,768); print('RS  %3d bx=%s %s'%(by0+k,bx,s[:70]))
