"""Regenerate Studio's GGX directional-albedo constants (NumPy, no Blender).

Integrates the actual correlated Smith/VNDF model in trace.wgsl with F=1.
Kulla/Conty compensation restores energy missing from single scattering.
This is numeric quadrature, not a copied renderer table or a binary asset.
"""
from pathlib import Path
import numpy as np


def generate(size=16, samples=65536):
    # Deterministic Hammersley quadrature over the visible-normal sampler.
    bits=np.arange(samples,dtype=np.uint32)
    reverse=np.zeros(samples,dtype=np.uint32)
    for i in range(32):
        reverse=(reverse<<1)|((bits>>i)&1)
    u=(np.arange(samples)+.5)/samples
    phi=2*np.pi*u
    vv=(reverse.astype(float)+.5)/2**32
    result=[]
    means=[]
    fresnel=[]
    fresnel_means=[]
    mus=np.linspace(0,1,size)
    for roughness in np.linspace(0,1,size):
        row=[]
        brow=[]
        a=max(roughness**2,1e-5)
        for mu in mus:
            mu=max(mu,1e-5)
            v=np.array([np.sqrt(1-mu*mu),0,mu])
            vs=np.array([v[0]*a,0,v[2]])
            vs/=np.linalg.norm(vs)
            z=(1-vv)*(1+vs[2])-vs[2]
            st=np.sqrt(np.maximum(1-z*z,0))
            h=np.array([st*np.cos(phi)+vs[0],st*np.sin(phi),z+vs[2]])
            h*=np.array([a,a,1])[:,None]
            h/=np.linalg.norm(h,axis=0)
            l=2*np.sum(v[:,None]*h,axis=0)*h-v[:,None]
            nl=np.maximum(l[2],1e-12)
            g1=2*mu/(mu+np.sqrt(a*a+(1-a*a)*mu*mu))
            g2=2*mu*nl/(nl*np.sqrt(mu*mu*(1-a*a)+a*a)+mu*np.sqrt(nl*nl*(1-a*a)+a*a))
            weight=np.where(l[2]>0,g2/g1,0)
            row.append(float(np.mean(weight)))
            brow.append(float(np.mean(weight*(1-np.clip(np.sum(v[:,None]*h,axis=0),0,1))**5)))
        if roughness == 0:
            row = [1.] * size
            brow = list((1-mus)**5)
        result.extend(row)
        fresnel.extend(brow)
        integrate = getattr(np, 'trapezoid', None) or np.trapz
        means.append(float(integrate(2*mus*np.array(row),mus)))
        fresnel_means.append(1/21 if roughness==0 else float(integrate(2*mus*np.array(brow),mus)))
    return result,means,fresnel,fresnel_means


def text():
    values,means,fresnel,fresnel_means=generate()
    rows=['// BEGIN GGX ENERGY LUT (generate_energy_lut.py, 16x16, 65536 samples/cell)',
          'const GGX_ENERGY = array<f32,256>(']
    rows += ['  '+', '.join(f'{v:.7f}' for v in values[i:i+16])+',' for i in range(0,256,16)]
    rows += [');','const GGX_ENERGY_AVG = array<f32,16>('+', '.join(f'{v:.7f}' for v in means)+');',
             'const GGX_FRESNEL_B = array<f32,256>(']
    rows += ['  '+', '.join(f'{v:.7f}' for v in fresnel[i:i+16])+',' for i in range(0,256,16)]
    rows += [');','const GGX_FRESNEL_B_AVG = array<f32,16>('+', '.join(f'{v:.7f}' for v in fresnel_means)+');',
             '// END GGX ENERGY LUT']
    return '\n'.join(rows)


if __name__=='__main__':
    print(text())
