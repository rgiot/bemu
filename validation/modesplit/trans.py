import sys
from cmp import load_ams, load_rs
from slots import slotstr, letter
def trans(img,y,x0,x1):
    bx,s=slotstr(img,y,x0,x1)
    if bx is None: return None
    first=((bx%8)-x0)%8  # slot grid start offset
    bslot=(bx-x0-first)//8
    out=[]; last=None
    for i,ch in enumerate(s):
        if ch in 'KB' and not (bslot<=i<bslot+2):
            if last is not None and ch!=last: out.append((i,last+'>'+ch))
            last=ch
    return bx,bslot,len(s),out
if __name__=='__main__':
    A=load_ams(sys.argv[1]); B=load_rs(sys.argv[2]); ay0,by0,n=int(sys.argv[3]),int(sys.argv[4]),int(sys.argv[5])
    for k in range(n):
        print('AMS',ay0+k,trans(A,ay0+k,0,768))
    for k in range(n):
        print('RS ',by0+k,trans(B,by0+k,96,832))
