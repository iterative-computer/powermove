# Still compatibility fixtures

All images here are synthetic RGB ramps created for these tests, with no
photography, user data, system ICC profiles, or captured screenshots.
`chromium.json` records premultiplied RGBA8 SHA-256 values from Electron 44
(Chromium 152.0.7977.54), through the real Powermove file importer and WebGL
upload. The opt-in `svgstills-capture.ts` harness produces these measurements.
Float64 and Lab TIFFs reject import. Deep lossless JPEGs import but their
RGBA8 upload fails; completed legacy exports show an opaque black sampler.
Those outcomes have no usable upload hash and are recorded separately.

PNG/TIFF ramps are 32×24: R=x/31, G=y/23, B=(x+y)/54, alpha=((x+y)%16)/15.
PNG samples use truncating integer quantization at the named depth. PQ/HLG
use CICP 9-16-0-full / 9-18-0-full. Chromaticity PNGs use gAMA 0.7 and
D65/P3 primaries. TIFFs were written with tifffile/imagecodecs: float16
Deflate with predictor 3, float32 LZW with predictor 3, planar RGB16,
CMYK-alpha, true 16×16 tiles, JPEG YCbCr, float64 and CIELab.
The unprofiled CMYK JPEG uses imagecodecs/libjpeg-turbo.

Lossless JPEG ramps are 64×48 with R=x/63, G=y/47, B=(x+y)/110, quantized
at 8/12/16 bits and encoded by libjpeg-turbo 3.2 `cjpeg -lossless 4`.
The two HDR AVIFs use FFmpeg's 64×48 `testsrc2`, 10-bit 4:2:0, SVT-AV1
preset 10 / CRF 25, with CICP primaries/transfer 1/16 or 9/18.

The `lossless2.jpg` through `lossless16.jpg` sweep uses the same ramp at
16×16, each stored sample precision, and `cjpeg -lossless 2`. Chromium keeps
2–8-bit samples on its 8-bit surface and exposes higher precisions as float
ImageBitmaps whose RGBA8 upload fails.

The metadata PNGs add valid mDCv/cLLi to the same HDR ramps (or to untagged
samples). `bt2020-sdr.avif` is the same FFmpeg AVIF recipe with CICP 9/1/9.
Arithmetic JPEGs use `cjpeg -arithmetic` with optional `-progressive`, at
8 and 12 bits. Chromium exposes those as float ImageBitmaps, so even the
8-bit arithmetic case has an incomplete black WebGL texture.
