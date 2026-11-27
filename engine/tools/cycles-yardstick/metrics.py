"""Scene-linear Rec.709 comparisons; no image-processing runtime dependencies."""
import numpy as np

LUMA = np.array([0.2126, 0.7152, 0.0722])


def neutral(rgb):
    """Same Khronos PBR Neutral curve as Studio post.wgsl, before sRGB."""
    rgb = np.maximum(rgb, 0).copy()
    x = rgb.min(axis=-1, keepdims=True)
    rgb -= np.where(x < .08, x-6.25*x*x, .04)
    peak = rgb.max(axis=-1, keepdims=True)
    new_peak = 1 - .24**2 / np.maximum(peak + .24 - .76, 1e-9)
    compressed = rgb * new_peak / np.maximum(peak, 1e-9)
    gray = 1 - 1 / (.15*(peak-new_peak)+1)
    compressed = compressed*(1-gray) + new_peak*gray
    return np.clip(np.where(peak < .76, rgb, compressed), 0, 1)


def srgb(rgb):
    rgb = np.maximum(rgb, 0)
    return np.where(rgb <= .0031308, rgb*12.92, 1.055*rgb**(1/2.4)-.055)


def decode(rgb):
    return np.where(rgb <= .04045, rgb/12.92, ((rgb+.055)/1.055)**2.4)


def lab(rgb):
    xyz = np.einsum("...c,dc->...d", rgb, np.array([[.4124564, .3575761, .1804375],
                          [.2126729, .7151522, .0721750],
                          [.0193339, .1191920, .9503041]]))
    xyz /= np.array([.95047, 1, 1.08883])
    delta = 6/29
    f = np.where(xyz > delta**3, np.cbrt(xyz), xyz/(3*delta**2) + 4/29)
    return np.stack([116*f[..., 1]-16, 500*(f[..., 0]-f[..., 1]),
                     200*(f[..., 1]-f[..., 2])], axis=-1)


def delta_e2000(a, b):
    """CIEDE2000, kL=kC=kH=1, Sharma/Wu/Dalal formulation."""
    l1, a1, b1 = np.moveaxis(np.asarray(a, dtype=float), -1, 0)
    l2, a2, b2 = np.moveaxis(np.asarray(b, dtype=float), -1, 0)
    cbar = (np.hypot(a1, b1) + np.hypot(a2, b2))/2
    g = .5*(1-np.sqrt(cbar**7/(cbar**7+25**7)))
    ap1, ap2 = (1+g)*a1, (1+g)*a2
    c1, c2 = np.hypot(ap1, b1), np.hypot(ap2, b2)
    h1 = np.mod(np.degrees(np.arctan2(b1, ap1)), 360)
    h2 = np.mod(np.degrees(np.arctan2(b2, ap2)), 360)
    h1, h2 = np.where(c1 == 0, 0, h1), np.where(c2 == 0, 0, h2)
    dl, dc = l2-l1, c2-c1
    dh = h2-h1
    dh = np.where(dh > 180, dh-360, np.where(dh < -180, dh+360, dh))
    dh = np.where(c1*c2 == 0, 0, dh)
    dH = 2*np.sqrt(c1*c2)*np.sin(np.radians(dh/2))
    lm, cm = (l1+l2)/2, (c1+c2)/2
    hm = np.where(np.abs(h1-h2) <= 180, (h1+h2)/2,
                  np.where(h1+h2 < 360, (h1+h2+360)/2, (h1+h2-360)/2))
    hm = np.where(c1*c2 == 0, h1+h2, hm)
    t = (1-.17*np.cos(np.radians(hm-30))+.24*np.cos(np.radians(2*hm))
         +.32*np.cos(np.radians(3*hm+6))-.20*np.cos(np.radians(4*hm-63)))
    sl = 1+.015*(lm-50)**2/np.sqrt(20+(lm-50)**2)
    sc, sh = 1+.045*cm, 1+.015*cm*t
    rt = -2*np.sqrt(cm**7/(cm**7+25**7))*np.sin(np.radians(60*np.exp(-((hm-275)/25)**2)))
    return np.sqrt(np.maximum((dl/sl)**2+(dc/sc)**2+(dH/sh)**2+rt*(dc/sc)*(dH/sh), 0))


def blur(a):
    weights = np.exp(-np.arange(-5, 6)**2/(2*1.5**2))
    weights /= weights.sum()
    for axis in [0, 1]:
        pad = [(0, 0)]*a.ndim
        pad[axis] = (5, 5)
        padded = np.pad(a, pad, mode='reflect')
        a = sum(weight * np.take(padded, np.arange(a.shape[axis])+i, axis=axis)
                for i, weight in enumerate(weights))
    return a


def ssim_map(a, b):
    """Wang SSIM: 11x11 Gaussian, sigma=1.5, L=1; computed on sRGB luma."""
    a, b = np.sum(a*LUMA, axis=-1), np.sum(b*LUMA, axis=-1)
    ma, mb = blur(a), blur(b)
    va, vb = np.maximum(blur(a*a)-ma*ma, 0), np.maximum(blur(b*b)-mb*mb, 0)
    covariance = blur(a*b)-ma*mb
    return ((2*ma*mb+.01**2)*(2*covariance+.03**2)
            / ((ma*ma+mb*mb+.01**2)*(va+vb+.03**2)))


def exposure_match(a, b, mask):
    la, lb = np.sum(a*LUMA, axis=-1), np.sum(b*LUMA, axis=-1)
    valid = mask & (la > .005) & (lb > .005)
    if not valid.any():
        return 1.
    # One scalar for the scene, never per-region or per-channel white balance.
    return float(np.clip(np.exp(np.median(np.log(lb[valid])-np.log(la[valid]))), 1/16, 16))


def score(a, b, masks, exposure=1, tone_map=True):
    """ΔE on common PBR Neutral display-linear RGB; raw errors stay linear HDR."""
    a, b = np.maximum(np.asarray(a, dtype=float), 0), np.maximum(np.asarray(b, dtype=float), 0)
    display_a, display_b = ((neutral(a*exposure), neutral(b)) if tone_map
                            else (np.clip(a*exposure,0,1),np.clip(b,0,1)))
    de = delta_e2000(lab(display_a), lab(display_b))
    ssim = ssim_map(srgb(display_a), srgb(display_b))
    result = {}
    for name, mask in masks.items():
        n = int(mask.sum())
        if not n:
            result[name] = dict(pixels=0, reason='region not visible / unsupported')
            continue
        lum_a, lum_b = np.sum(a[mask]*LUMA, axis=-1), np.sum(b[mask]*LUMA, axis=-1)
        result[name] = dict(pixels=n, delta_e_mean=float(de[mask].mean()),
                            delta_e_p50=float(np.percentile(de[mask], 50)),
                            delta_e_p95=float(np.percentile(de[mask], 95)),
                            delta_e_p99=float(np.percentile(de[mask], 99)),
                            ssim=float(ssim[mask].mean()),
                            linear_rgb_mae=np.abs(a[mask]*exposure-b[mask]).mean(axis=0).tolist(),
                            linear_luminance_mae=float(np.abs(lum_a*exposure-lum_b).mean()),
                            raw_luminance_ratio=float(lum_a.mean()/max(lum_b.mean(), 1e-9)))
    return result
