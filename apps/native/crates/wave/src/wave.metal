// Port of distortion-waves 0.1.0 (Jakub Antalik), renderer.ts MAIN_FRAG.
// Copyright (c) 2026 Jakub Antalik. MIT; see ../LICENSE.
// Calculations and blending stay in display-encoded sRGB, as in WebGL.
#include <metal_stdlib>
using namespace metal;
struct Uniforms { float v[256]; };
constexpr sampler waveSampler(coord::normalized, address::clamp_to_edge, filter::linear, mip_filter::linear);
#define uSize (float2(uniforms.v[0], uniforms.v[1]))
#define uPx (float2(uniforms.v[2], uniforms.v[3]))
#define uF (uniforms.v + 4)
#define uC0 tex[0]
#define uC1 tex[1]
#define uC2 tex[2]
#define uR0 tex[3]
#define uR1 tex[4]
#define uR2 tex[5]
#define uContent (int(uniforms.v[23]))
#define uRevealed (int(uniforms.v[24]))
#define uLens (int(uniforms.v[25]))
#define uLive (int(uniforms.v[26]))
#define uSame (int(uniforms.v[27]))
#define uRound (int(uniforms.v[28]))
#define uFloor (uniforms.v[29])
#define uAmplitude (uniforms.v[30])
#define uLensWidth (uniforms.v[31])
#define uAberration (uniforms.v[32])
#define uBlur (uniforms.v[33])
#define uBlurAhead (uniforms.v[34])
#define uBlurBehind (uniforms.v[35])
#define uNear (uniforms.v[36])
#define uPalCount (int(uniforms.v[64]))
#define uAhead (float3(uniforms.v[65], uniforms.v[66], uniforms.v[67]))
#define uBehind (float3(uniforms.v[68], uniforms.v[69], uniforms.v[70]))
#define uLead (uniforms.v[71])
#define uTail (uniforms.v[72])
#define uStrength (uniforms.v[73])
#define uRimWidth (uniforms.v[74])
#define uRimSoftness (uniforms.v[75])
#define uCorner (uniforms.v[76])
#define uUneven (uniforms.v[77])
#define uPulse (uniforms.v[78])
#define uHairW (uniforms.v[79])
#define uHairO (uniforms.v[80])
#define uHairWhite (uniforms.v[81])

constant float coverZone = 60.0;
constant float PI = 3.14159265;

struct Field { float d; float2 n; float2 q; float2 raw; float g; };

float heldAt(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, float2 p) {
  if (uF[16] <= 0.0) return 1.0;
  return smoothstep(uF[16] * 0.9, uF[16] * 1.8, length(p - float2(uF[0], uF[1])));
}

Field fieldAt(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, float2 p) {
  float2 raw = p - float2(uF[0], uF[1]);
  float cs = cos(uF[14]), sn = sin(uF[14]);
  float2 q = float2(cs * raw.x + sn * raw.y, -sn * raw.x + cs * raw.y);
  float spine = uF[2], radius = uF[3], squash = uF[4], time = uF[5], swell = uF[7];
  float mx = q.x * squash;
  float2 v = float2(max(abs(mx) - spine, 0.0), q.y);
  float l = max(length(v), 1e-3);
  float2 grad = float2(v.x * sign(q.x) * squash, v.y) / l;
  float g = max(length(grad), 1e-3);
  float bend;
  if (uF[15] > 0.5) {
    float a = atan2(q.y, q.x);
    bend = 0.62 * sin(2.0 * a + time * 1.1) + 0.38 * sin(3.0 * a - time * 1.5 + 1.7);
  } else {
    float s = mx * 0.0058;
    bend = 0.62 * sin(s + time * 1.1) + 0.38 * sin(s * 2.1 - time * 1.5 + 1.7);
  }
  float2 n = grad / g;
  Field o;
  o.d = (l - radius) / g + swell * bend;
  o.n = float2(cs * n.x - sn * n.y, sn * n.x + cs * n.y);
  o.q = q;
  o.raw = raw;
  o.g = g;
  return o;
}

/* The progressive blur, as the stacked blurred copies: near over sharp, far
   over that, each taking over where the last ends.

   Every read names its level. These calls sit inside branches that differ
   from one pixel to the next, where the derivatives a plain texture() picks
   its mip level from are undefined: along each branch's boundary it would
   read a far smaller level, and draw the boundary as a thin dotted line. */
/* Premultiplied: a screen with clear parts (a picture over nothing) keeps
   them clear through the lens and the blur; an opaque one reads as before. */
float4 side(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, texture2d<float> a, texture2d<float> b, texture2d<float> c, float2 s, float tn, float tf) {
  float2 uv = clamp(s, float2(0.5), uSize - 0.5) / uSize;
  float4 col = a.sample(waveSampler, uv, level(0.0));
  if (tn > 0.0) col = mix(col, b.sample(waveSampler, uv, level(0.0)), tn);
  if (tf > 0.0) col = mix(col, c.sample(waveSampler, uv, level(0.0)), tf);
  return col;
}

constant float3 spectrum[5] = {
  float3(0.571, 0.0, 0.0), float3(0.343, 0.206, 0.0), float3(0.086, 0.588, 0.086), float3(0.0, 0.206, 0.343), float3(0.0, 0.0, 0.571)
};

float4 lensed(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, int which, float2 s, float2 split, float tn, float tf) {
  if (dot(split, split) < 0.0625) {
    return which == 0 ? side(uniforms, tex, uC0, uC1, uC2, s, tn, tf) : side(uniforms, tex, uR0, uR1, uR2, s, tn, tf);
  }
  float4 o = float4(0.0);
  for (int i = 0; i < 5; i++) {
    float along = float(i) / 4.0 * 2.0 - 1.0;
    float4 c = which == 0 ? side(uniforms, tex, uC0, uC1, uC2, s + split * along, tn, tf) : side(uniforms, tex, uR0, uR1, uR2, s + split * along, tn, tf);
    // Each channel's weights sum to one across the spread; coverage is
    // their mean.
    o += float4(c.rgb * spectrum[i], c.a * 0.2);
  }
  return o;
}

float continuousRectSDF(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, float2 p, float2 halfSize, float radius) {
  float a = min(radius * 1.528665, min(halfSize.x, halfSize.y));
  float2 q = abs(p) - (halfSize - a);
  if (q.x <= 0.0 || q.y <= 0.0) return max(abs(p.x) - halfSize.x, abs(p.y) - halfSize.y);
  const float n = 3.26;
  float2 u = q / a;
  float norm = pow(pow(u.x, n) + pow(u.y, n), 1.0 / n);
  float2 gg = pow(u / max(norm, 1e-4), float2(n - 1.0));
  return (norm - 1.0) * a / max(length(gg), 1e-3);
}

/* The area's edge: the continuous corners of an iOS screen, or the circular
   ones CSS draws. */
float edgeSDF(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, float2 p, float2 halfSize, float radius) {
  if (uRound == 0) return continuousRectSDF(uniforms, tex, p, halfSize, radius);
  float r = min(radius, min(halfSize.x, halfSize.y));
  float2 q = abs(p) - (halfSize - r);
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

float2 perimeterAt(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, float2 p, float2 halfSize, float r) {
  float ex = max(halfSize.x - r, 0.0);
  float ey = max(halfSize.y - r, 0.0);
  float arc = 0.5 * PI * r;
  float P = 4.0 * (ex + ey + arc);
  float ax = abs(p.x), ay = abs(p.y);
  if (ax > ex && ay > ey) {
    float2 v = p - float2(sign(p.x) * ex, sign(p.y) * ey);
    float a, base;
    if (p.x >= 0.0 && p.y < 0.0) { a = atan2(v.x, -v.y); base = ex; }
    else if (p.x >= 0.0) { a = atan2(v.y, v.x); base = ex + arc + 2.0 * ey; }
    else if (p.y >= 0.0) { a = atan2(-v.x, v.y); base = 3.0 * ex + 2.0 * arc + 2.0 * ey; }
    else { a = atan2(-v.y, -v.x); base = 3.0 * ex + 3.0 * arc + 4.0 * ey; }
    return float2(base + r * clamp(a, 0.0, 0.5 * PI), P);
  }
  bool sideHit = ax > ex ? true : (ay > ey ? false : (halfSize.x - ax) < (halfSize.y - ay));
  if (sideHit) return float2(p.x >= 0.0 ? ex + arc + (p.y + ey) : 3.0 * ex + 3.0 * arc + 2.0 * ey + (ey - p.y), P);
  return float2(p.y < 0.0 ? (p.x >= 0.0 ? p.x : P + p.x) : ex + 2.0 * arc + 2.0 * ey + (ex - p.x), P);
}

float rimThickness(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, float s, float P) {
  float t = 0.08;
  for (int i = 0; i < 7; i++) {
    float ds = s - float3(uniforms.v[82+(i)*3],uniforms.v[82+(i)*3+1],uniforms.v[82+(i)*3+2]).x;
    ds -= P * floor(ds / P + 0.5);
    float k = ds / float3(uniforms.v[82+(i)*3],uniforms.v[82+(i)*3+1],uniforms.v[82+(i)*3+2]).y;
    t += float3(uniforms.v[82+(i)*3],uniforms.v[82+(i)*3+1],uniforms.v[82+(i)*3+2]).z * exp(-k * k);
  }
  return t / 1.0498;
}

float dither(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, float2 p) { return fract(52.9829189 * fract(dot(p, float2(0.06711056, 0.00583715)))) - 0.5; }

float3 paletteAt(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, float u) {
  float x = fract(u) * float(uPalCount);
  int i = min(int(x), uPalCount - 1);
  int j = i + 1 == uPalCount ? 0 : i + 1;
  float m = fract(x);
  m = m * m * (3.0 - 2.0 * m);
  return mix(float3(uniforms.v[37+(i)*3],uniforms.v[37+(i)*3+1],uniforms.v[37+(i)*3+2]), float3(uniforms.v[37+(j)*3],uniforms.v[37+(j)*3+1],uniforms.v[37+(j)*3+2]), m);
}

float3 oklabToSRGB(constant Uniforms &uniforms, array<texture2d<float>, 6> tex, float3 c) {
  float l = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
  float m = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
  float s = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;
  l = l * l * l; m = m * m * m; s = s * s * s;
  float3 lin = clamp(float3(
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s
  ), 0.0, 1.0);
  return mix(12.92 * lin, 1.055 * pow(lin, float3(1.0 / 2.4)) - 0.055, step(float3(0.0031308), lin));
}

float4 wave_picture(float4 position, constant Uniforms &uniforms, array<texture2d<float>,6> tex) {
  float4 gl_FragCoord = float4(position.x, uPx.y - position.y, 0.0, 1.0);
  float2 p = float2(gl_FragCoord.x, uPx.y - gl_FragCoord.y) * (uSize.x / uPx.x);

  if (uniforms.v[103] >= 0.0) {
    return mix(uC0.sample(waveSampler,p/uSize,level(0)),uR0.sample(waveSampler,p/uSize,level(0)),uniforms.v[103]);
  }
  if (uLive == 0) {
    return uRevealed == 1 ? uR0.sample(waveSampler, p / uSize, level(0.0)) : uC0.sample(waveSampler, p / uSize, level(0.0));
  }

  Field w = fieldAt(uniforms, tex, p);
  float energy = uF[6], gravity = uF[10];
  float held = heldAt(uniforms, tex, p);

  // ─── The two screens, through the swell (waveLens) ───
  float2 s = p;
  float2 split = float2(0.0);
  if (uLens == 1) {
    float u = w.d / (uLensWidth * (w.d > 0.0 ? 0.85 : 1.5));
    float lift = exp(-u * u);
    float r = max(length(w.raw), 1.0);
    float2 swollen = w.n * energy * uAmplitude * lift * held;
    float2 inward = w.raw / r;
    s = p - swollen + inward * gravity * exp(-r / 900.0) * held;
    split = (swollen - inward * gravity * exp(-r / 240.0) * held) * uAberration;
  }
  // The progressive blur, read where the pixel is (waveBlurMask).
  float bu = w.d / (w.d > 0.0 ? uBlurAhead : uBlurBehind);
  float radius = energy * uBlur * exp(-bu * bu) * held;
  float tn = clamp(radius / max(uNear, 1e-3), 0.0, 1.0);
  tn = tn * tn * (3.0 - 2.0 * tn);
  float tf = clamp((radius - uNear) / max(uBlur - uNear, 1e-3), 0.0, 1.0);
  tf = tf * tf * (3.0 - 2.0 * tf);

  float cover = 1.0;
  if (uContent == 1 && uF[17] > 0.5) cover = smoothstep(-coverZone, coverZone, w.d);
  if (uContent == 0) cover = 0.0;

  // How much of this pixel the canvas owns. All of it when opaque; over a
  // page, only what the wave changes: where the lens moves, splits or blurs
  // the screen, and where the revealed screen has taken over from the
  // content the page is still showing. Each reaches 1 well before its
  // effect can be seen, so the hand-over to the page has no seam.
  float own = uFloor;
  if (own < 1.0) {
    float moved = max(smoothstep(0.03, 0.4, length(s - p)), smoothstep(0.03, 0.4, length(split)));
    own = max(own, max(moved, smoothstep(0.0, 0.6, radius)));
    if (uSame == 0 && uContent == 1) own = max(own, 1.0 - smoothstep(0.9, 1.0, cover));
  }

  float4 col = float4(0.0);
  if (own > 0.0) {
    if (uSame == 1) {
      col = lensed(uniforms, tex, 0, s, split, tn, tf);
    } else {
      bool showRevealed = uRevealed == 1 && w.d <= coverZone + 6.0 && cover < 1.0;
      if (showRevealed) col = lensed(uniforms, tex, 1, s, split, tn, tf);
      if (cover > 0.0) col = mix(col, lensed(uniforms, tex, 0, s, split, tn, tf), cover);
    }
  }

  // ─── The colour (waveBand) ───
  float d = w.d, time = uF[5], glow = uF[11], rim = uF[13];
  float2 c = p - uSize * 0.5;
  float inset = max(-edgeSDF(uniforms, tex, c, uSize * 0.5, uCorner), 0.0);
  bool nearEdge = rim > 0.0 && inset < max(uRimWidth * 2.2 * (1.0 + 1.9 * uUneven), uHairW * 0.5 + 0.75);
  if (glow <= 0.0 && !nearEdge) {
    return float4((col.rgb + col.a * dither(uniforms, tex, gl_FragCoord.xy) / 255.0) * own, col.a * own);
  }
  bool ring = uF[15] > 0.5;
  float turn = atan2(w.q.y, w.q.x);
  float along = w.q.x * 0.0068;
  float lobes = ring
    ? 0.5 + 0.5 * sin(2.0 * turn + time * 1.4) * cos(turn - time * 0.7 + 1.2)
    : 0.5 + 0.5 * sin(along * 1.3 + time * 1.4) * cos(along * 0.6 - time * 0.7 + 1.2);
  float bandA = 0.0;
  if (glow > 0.0 && d < uLead) {
    if (d >= 0.0) {
      float k = 1.0 - d / uLead;
      bandA = k * k * k * (k * (k * 6.0 - 15.0) + 10.0);
    } else {
      bandA = pow(1.0 - clamp(-d / (uTail * mix(0.6, 1.3, lobes)), 0.0, 1.0), 2.3);
    }
    bandA *= glow * uStrength * mix(0.6, 1.0, exp(-inset / 140.0)) * held;
  }
  float aroundA = atan2(c.y / uSize.y, c.x / uSize.x);
  float rimA = 0.0;
  float hairA = 0.0;
  float shimmer = 0.5;
  if (nearEdge) {
    float2 edge = perimeterAt(uniforms, tex, c, uSize * 0.5, uCorner);
    float thickness = 1.0;
    if (uUneven > 0.0) thickness = max(1.0 + (rimThickness(uniforms, tex, edge.x, edge.y) - 1.0) * uUneven, 0.04);
    float hk = inset / (uRimWidth * thickness);
    float hug = exp(-hk * hk);
    float reached = 1.0 - smoothstep(-uLead * 0.7, uLead * 0.5, d);
    float kept = smoothstep(-uRimSoftness, uRimSoftness, d + (uF[3] - uF[12]) / w.g);
    shimmer = 0.5 + 0.5 * sin(time * 3.4 - edge.x / edge.y * 4.0 * PI);
    rimA = hug * reached * kept * rim * mix(1.0, 0.7 + 0.3 * shimmer, uPulse);
    if (uHairW > 0.0) {
      float line = 1.0 - smoothstep(uHairW * 0.5, uHairW * 0.5 + 0.75, inset);
      float hairKept = smoothstep(-uRimSoftness, uRimSoftness, d + (uF[3] - uF[18]) / w.g);
      hairA = line * reached * hairKept * rim * uHairO
        * mix(0.5, 1.0, smoothstep(0.1, 1.5, thickness))
        * mix(1.0, 0.55 + 0.45 * shimmer, uPulse);
    }
  }
  float a = 1.0 - (1.0 - bandA) * (1.0 - rimA);
  float total = 1.0 - (1.0 - a) * (1.0 - hairA);
  if (total <= 0.0005) {
    return float4((col.rgb + col.a * dither(uniforms, tex, gl_FragCoord.xy) / 255.0) * own, col.a * own);
  }
  float flow = 0.62 + (ring ? turn / (2.0 * PI) : along * 0.2) + time * 0.13 + d * 0.0015 + (ring ? 0.0 : 0.05 * sin(w.q.y * 0.009 + time));
  float3 lab = paletteAt(uniforms, tex, flow);
  lab = mix(lab, uAhead, 0.5 * exp(-pow((d - 30.0) / 50.0, 2.0)));
  lab = mix(lab, uBehind, 0.5 * exp(-pow((d + 84.0) / 72.0, 2.0)));
  lab.yz *= 1.22;
  lab = mix(lab, float3(0.99, 0.0, 0.0), 0.7 * exp(-pow((d + 8.0) / 26.0, 2.0)));
  float3 rimLab = paletteAt(uniforms, tex, aroundA / (2.0 * PI) + time * (0.12 + 0.25 * uPulse) + inset * 0.0012);
  rimLab.yz *= 1.1;
  lab = mix(lab, rimLab, rimA / (rimA + bandA + 1e-4));
  float3 rgb = oklabToSRGB(uniforms, tex, lab);
  if (hairA > 0.0) {
    float3 hair = mix(oklabToSRGB(uniforms, tex, rimLab), float3(1.0), uHairWhite * (0.5 + 0.5 * shimmer));
    rgb = (rgb * a * (1.0 - hairA) + hair * hairA) / total;
  }
  float noise = dither(uniforms, tex, gl_FragCoord.xy) * 2.0 / 255.0;
  // The colour over the screens, as the band view over the transition —
  // premultiplied, so it lies as well over the page where the canvas is clear.
  float over = clamp(total + noise, 0.0, 1.0);
  return float4(col.rgb * own * (1.0 - over) + clamp(rgb + noise, 0.0, 1.0) * over, col.a * own * (1.0 - over) + over);
}
float cornerCoverage(float2 p, constant Uniforms &uniforms) {
  float2 halfSize=uSize*0.5;float r=min(uCorner,min(halfSize.x,halfSize.y));
  float2 q=abs(p-halfSize)-(halfSize-r);
  float distance=length(max(q,0.0))+min(max(q.x,q.y),0.0)-r;
  return clamp(0.5-distance*(uPx.x/uSize.x),0.0,1.0);
}
fragment float4 wave_fragment(float4 position [[position]], constant Uniforms &uniforms [[buffer(0)]], array<texture2d<float>,6> tex [[texture(0)]]) {
  return wave_picture(position,uniforms,tex)*cornerCoverage(position.xy*(uSize.x/uPx.x),uniforms);
}

vertex float4 wave_vertex(uint id [[vertex_id]]) {
  const float2 p[4] = {float2(-1,-1),float2(1,-1),float2(-1,1),float2(1,1)};
  return float4(p[id],0,1);
}
fragment float4 wave_copy(float4 p [[position]], texture2d<float> input [[texture(0)]], constant float4 &c [[buffer(0)]]) {
  return input.sample(waveSampler, p.xy / c.xy, level(c.z));
}
fragment float4 wave_blur(float4 p [[position]], texture2d<float> input [[texture(0)]], constant float4 &c [[buffer(0)]]) {
  float2 uv = p.xy / c.xy;
  float2 step = c.w == 0 ? float2(1 / c.x,0) : float2(0,1 / c.y);
  float4 acc = float4(0); float sum = 0;
  for (float i=-48; i<=48; i+=1) {
    if (abs(i)>ceil(c.z*3)) continue;
    float w=exp(-0.5*i*i/(c.z*c.z));
    acc += input.sample(waveSampler,uv+step*i,level(0))*w; sum+=w;
  }
  return acc/sum;
}
struct Dot { float4 position; };
struct DotOut { float4 position [[position]]; float size [[point_size]]; float alpha; float edge; };
vertex DotOut wave_dot_vertex(uint id [[vertex_id]], constant float4 *dots [[buffer(0)]], constant float4 &c [[buffer(1)]]) {
  float4 d=dots[id];
  DotOut o; o.position=float4(d.x/c.x*2-1,1-d.y/c.y*2,0,1); o.size=c.z;
  o.alpha=fract(d.z); o.edge=floor(d.z)/10*c.w*0.5; return o;
}
fragment float4 wave_dot_fragment(DotOut d [[stage_in]], float2 p [[point_coord]], constant Uniforms &uniforms [[buffer(0)]]) {
  float a=d.alpha*clamp(d.edge-length(p-0.5)*d.size+0.5,0.0,1.0); return float4(a)*cornerCoverage(d.position.xy*(uSize.x/uPx.x),uniforms);
}
