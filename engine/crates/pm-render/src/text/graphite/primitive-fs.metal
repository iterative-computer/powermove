// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
// Dawn/Tint 571752c9cb3ed36a3194854374984427794c4dda; see tools/graphite-oracle/DAWN-LICENSE.
#pragma METAL fp math_mode(relaxed)
#include <metal_stdlib>
using namespace metal;

template<typename T, size_t N>
struct tint_array {
  const constant T& operator[](size_t i) const constant { return elements[i]; }
  device T& operator[](size_t i) device { return elements[i]; }
  const device T& operator[](size_t i) const device { return elements[i]; }
  thread T& operator[](size_t i) thread { return elements[i]; }
  const thread T& operator[](size_t i) const thread { return elements[i]; }
  threadgroup T& operator[](size_t i) threadgroup { return elements[i]; }
  const threadgroup T& operator[](size_t i) const threadgroup { return elements[i]; }
  T elements[N];
};

struct tint_struct_1 {
  /* 0x0000 */ tint_array<float4, 9> tint_member_1;
};

struct tint_struct {
  const constant tint_struct_1* tint_member;
};

struct tint_struct_2 {
  float4 tint_member_2 [[color(0)]];
};

float4 v(tint_struct v_1) {
  return (*v_1.tint_member).tint_member_1[8];
}

fragment tint_struct_2 tint_entry_point(const constant tint_struct_1* v_3 [[buffer(0)]]) {
  tint_struct const v_4 = tint_struct{.tint_member=v_3};
  tint_struct_2 v_5 = {};
  v_5.tint_member_2 = v(v_4);
  return v_5;
}
