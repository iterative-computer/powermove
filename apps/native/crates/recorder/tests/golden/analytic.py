#!/usr/bin/env python3
"""Independent synthetic RGBA PNG oracles using elementary area/filter arithmetic.
Circle coverage integrates exact horizontal disk intersections at 16384 midpoint
ordinates per pixel; the bound is comfortably below 0.5/255 for these fixtures.
No recorder/engine geometry, browser output, installed fonts or user data.
"""
from pathlib import Path
import hashlib
import json
import math
import struct
import zlib

def png(path, width, height, rgba):
    def chunk(kind, data):
        return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
    raw=b''.join(b'\0'+bytes(rgba[y*width*4:(y+1)*width*4]) for y in range(height))
    path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',width,height,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw,9))+chunk(b'IEND',b''))

def main():
    dest=Path(__file__).resolve().parent
    n=16384
    values=[]
    for y in range(8):
        for x in range(8):
            area=0.
            for i in range(n):
                dy=y+(i+.5)/n-4.
                if abs(dy)<3.25:
                    rx=math.sqrt(3.25**2-dy**2)
                    area+=max(0.,min(x+1.,4.+rx)-max(x,4.-rx))/n
            values.extend([0,0,0,round(area*255)])
    png(dest/'analytic-circle.png',8,8,values)
    # HTML drawImage maps destination centers through inverse scale, clamps
    # the four taps to the crop, and interpolates premultiplied source pixels.
    values=[]
    source=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.],[1.,1.,1.]]
    for y in range(12):
        for x in range(12):
            sx=(x+.5)/6.-.5;sy=(y+.5)/6.-.5
            ix=math.floor(sx);iy=math.floor(sy);fx=sx-ix;fy=sy-iy
            def get(x,y):return source[min(1,max(0,y))*2+min(1,max(0,x))]
            taps=[get(ix,iy),get(ix+1,iy),get(ix,iy+1),get(ix+1,iy+1)]
            weights=[(1-fx)*(1-fy),fx*(1-fy),(1-fx)*fy,fx*fy]
            values.extend([round(255*sum(t[j]*w for t,w in zip(taps,weights))) for j in range(3)]+[255])
    png(dest/'analytic-bilinear.png',12,12,values)
    # Rectangle at (.25,.25) has separable exact covered area per pixel.
    values=[]
    for y in range(2):
        for x in range(2):
            a=max(0,min(x+1,1.25)-max(x,.25))*max(0,min(y+1,1.25)-max(y,.25))
            values.extend([255,0,0,round(a*255)])
    png(dest/'analytic-rect.png',2,2,values)
    # Invert y(t) independently for two arches: cubic and quadratic. At each
    # ordinate the filled region is the horizontal interval between its roots.
    def integrate(width, height, spans):
        values=[]
        for y in range(height):
            areas=[0.]*width
            for i in range(n):
                for lo,hi in spans(y+(i+.5)/n):
                    for x in range(max(0,math.floor(lo)), min(width,math.ceil(hi))):
                        areas[x]+=max(0.,min(x+1.,hi)-max(x,lo))/n
            for area in areas: values.extend([0,0,0,round(area*255)])
        return values
    def arches(y):
        out=[]
        if 3.25<y<10:
            t=(27-math.sqrt(729-108*(10-y)))/54
            x=1+42*t*t-28*t*t*t
            out.append((x,16-x))
        if 3<y<10:
            t=(28-math.sqrt(784-112*(10-y)))/56
            x=17+14*t
            out.append((x,48-x))
        return out
    png(dest/'analytic-beziers.png',32,12,integrate(32,12,arches))
    def rounded(y):
        if not 1<y<22: return []
        dy=max(5-y,y-18,0)
        inset=4-math.sqrt(max(0,16-dy*dy)) if dy else 0
        return [(1+inset,15-inset)]
    png(dest/'analytic-roundrect.png',16,24,integrate(16,24,rounded))
    def ellipse(y):
        dy=y-6;cos=math.cos(.35);sin=math.sin(.35)
        a=cos*cos/25+sin*sin/9
        b=2*cos*sin*dy*(1/25-1/9)
        c=dy*dy*(sin*sin/25+cos*cos/9)-1
        disc=b*b-4*a*c
        return [] if disc<=0 else [(8+(-b-math.sqrt(disc))/(2*a),8+(-b+math.sqrt(disc))/(2*a))]
    png(dest/'analytic-ellipse.png',16,12,integrate(16,12,ellipse))
    manifest={'generator':'analytic.py','sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(dest.glob('analytic-*.png'))}}
    (dest/'analytic-manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
    print('Generated 6 independent analytic PNG references')
if __name__=='__main__':main()
