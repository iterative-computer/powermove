"""Rec.709 spectral quadrature from Wyman et al. JCGT 2(2), equation 4.

https://jcgt.org/published/0002/02/01/
This is a color-matching fit, never a reference-renderer/image lookup.
"""
import numpy as np


def response():
    wavelength = np.arange(390., 771., 20.)
    def gaussian(center, left, right):
        t = (wavelength-center)*np.where(wavelength<center,left,right)
        return np.exp(-.5*t*t)
    x = (1.056*gaussian(599.8,.0264,.0323) + .362*gaussian(442.,.0624,.0374)
         - .065*gaussian(501.1,.049,.0382))
    y = .821*gaussian(568.8,.0213,.0247) + .286*gaussian(530.9,.0613,.0322)
    z = 1.217*gaussian(437.,.0845,.0278) + .681*gaussian(459.,.0385,.0725)
    xyz_to_rgb = np.array([[3.2404542,-1.5371385,-.4985314],
                           [-.969266,1.8760108,.041556],
                           [.0556434,-.2040259,1.0572252]])
    weights = np.stack([x,y,z],1) @ xyz_to_rgb.T
    # White balance each channel: a constant spectrum must stay neutral.
    return weights/weights.sum(axis=0)


def text():
    return ('const FILM_RESPONSE = array<vec3f,20>(\n'
            + ''.join('  vec3f('+', '.join(f'{v:.9f}' for v in row)+'),\n' for row in response())
            + ');')


if __name__ == '__main__':
    print(text())
