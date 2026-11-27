"""Integrate Studio's Charlie + soft visibility closure; no reference images.

Cosine-weighted deterministic hemisphere quadrature, 65536 samples/cell.
The reciprocal layer returns the base's internally reflected energy and
reserves the sheen's directional albedo before adding that sheen closure.
"""
from functools import lru_cache
import numpy as np


@lru_cache(maxsize=1)
def tables():
    side = 256
    u = (np.arange(side)+.5)/side
    radius = np.sqrt(np.repeat(u,side))
    phi = np.tile(u,side)*2*np.pi
    lx,ly,lz = radius*np.cos(phi),radius*np.sin(phi),np.sqrt(1-radius*radius)
    result = np.zeros((16,16))
    for y,r in enumerate(np.linspace(0,1,16)):
        inv = 1/max(r,.07)
        for x,nv in enumerate(np.linspace(0,1,16)):
            nv = max(nv,1e-6)
            hx,hy,hz = lx+np.sqrt(1-nv*nv),ly,lz+nv
            nh2 = hz*hz/(hx*hx+hy*hy+hz*hz)
            d = (2+inv)*np.maximum(0,1-nh2)**(inv/2)/(2*np.pi)
            visibility = 1/(4*(lz+nv-lz*nv))
            result[y,x] = np.mean(np.pi*d*visibility)
    cosine = np.linspace(0,1,16)
    integrand=result*2*cosine
    average=np.sum((integrand[:,:-1]+integrand[:,1:])*.5*np.diff(cosine),axis=1)
    return result,average


def text():
    values,average = tables()
    return ('// BEGIN SHEEN ENERGY LUT (generate_sheen_lut.py, 65536 samples/cell)\n'
            + 'const SHEEN_ENERGY = array<f32,256>(\n'
            + ''.join('  '+', '.join(f'{v:.8f}' for v in row)+',\n' for row in values)
            + ');\nconst SHEEN_ENERGY_AVG = array<f32,16>('
            + ', '.join(f'{v:.8f}' for v in average)+');\n'
            + 'const SHEEN_NORMALIZATION = array<f32,16>('
            + ', '.join(f'{v:.8f}' for v in np.maximum(1,values.max(axis=1)))+');\n// END SHEEN ENERGY LUT')


if __name__ == '__main__':
    print(text())
