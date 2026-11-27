"""Pure parameter conversions for the optional external reference."""
import math


def anisotropic_widths(roughness, anisotropy):
    k = abs(anisotropy)
    return roughness*(1-k*k)**.25, min(2*k/(.9*(1+k)),1.)


def conductor_ior(f0):
    """Define the filmed RGB substrate by F0 and Schlick at acos(1/7).

    The engine solves exact conductor Fresnel for its complex IOR. Principled
    instead reinterprets F82 as Gulbrandsen edge tint, a different material.
    """
    f0 = min(max(f0,1e-5),.9999)
    root = math.sqrt(f0)
    lo, hi = (1-root)/(1+root), (1+root)/(1-root)
    target = f0+(1-f0)*(6/7)**5
    for _ in range(12):
        n = (lo+hi)/2
        k = math.sqrt(max(0,(f0*(n+1)**2-(n-1)**2)/(1-f0)))
        # Complex Fresnel is independent of the engine's real-valued formula.
        eta = complex(n,k)
        cos = 1/7
        q = (eta*eta-(1-cos*cos))**.5
        rs = (cos-q)/(cos+q)
        rp = (eta*eta*cos-q)/(eta*eta*cos+q)
        value = (abs(rs)**2+abs(rp)**2)/2
        if value > target:
            lo = n
        else:
            hi = n
    n = (lo+hi)/2
    return n, math.sqrt(max(0,(f0*(n+1)**2-(n-1)**2)/(1-f0)))
