// Small crosshair shape keeps the cursor fragment regression self-contained.
float cursorShape(vec2 p, int t) {
  p -= vec2(12.0);
  return min(max(abs(p.x)-1.0, abs(p.y)-8.0), max(abs(p.x)-8.0, abs(p.y)-1.0));
}
void main(){
  // 24-unit cursor art board inside a 32-unit layer; hotspot sits near the top-left.
  vec2 p = uv * 32.0;
  float s = max(u_size, 0.01) * (1.0 - 0.10 * clamp(u_press, 0.0, 1.0));
  vec2 hot = vec2(1.0, 1.0);
  p = (p - hot) / s + hot;

  int t = int(floor(clamp(u_cursor, 0.0, 10.0) + 0.5));
  float d = cursorShape(p, t);

  float aa = max(fwidth(d), 1e-4);
  float sw = max(u_outline, 0.0);

  bool nativeInvert = (t == 1 || t == 5);
  bool invert = nativeInvert != u_swap;
  vec3 body = invert ? u_edge : u_fill;
  vec3 rim  = invert ? u_fill : u_edge;

  float inner = 1.0 - smoothstep(-aa, aa, d);
  float outer = 1.0 - smoothstep(-aa, aa, d - sw);

  vec3 col = mix(rim, body, inner);
  float a = outer;

  // Soft drop shadow behind the glyph, offset down-right like the system cursor.
  float sAmt = clamp(u_shadow, 0.0, 1.0);
  if (sAmt > 0.001) {
    float ds = cursorShape(p - vec2(0.6, 0.9) / max(s, 0.01), t) - sw;
    float sa = (1.0 - smoothstep(-1.2, 1.2, ds)) * 0.45 * sAmt;
    float outA = a + sa * (1.0 - a);
    // Shadow colour is black, so the composited colour is just the glyph's own
    // contribution divided by the new coverage (straight alpha).
    col = col * a / max(outA, 1e-4);
    a = outA;
  }

  a *= clamp(u_opacity, 0.0, 1.0);
  fragColor = vec4(col, a);
}
