// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
// Dawn/Tint 571752c9cb3ed36a3194854374984427794c4dda; see tools/graphite-oracle/DAWN-LICENSE.
#pragma METAL fp math_mode(relaxed)
#include <metal_stdlib>
using namespace metal;

volatile constexpr constant uint tint_volatile_zero = 0u;

struct tint_struct {
  uint tint_member;
  float4 tint_member_1;
  float2 tint_member_2;
  float2 tint_member_3;
  float tint_member_4;
};

struct tint_struct_1 {
  float4 tint_member_5;
};

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

struct tint_struct_4 {
  /* 0x0000 */ float4 tint_member_8;
  /* 0x0010 */ float4x4 tint_member_9;
  /* 0x0050 */ float4x4 tint_member_10;
  /* 0x0090 */ float2 tint_member_11;
  /* 0x0098 */ float2 tint_member_12;
};

struct tint_struct_3 {
  /* 0x0000 */ tint_array<tint_struct_4, 1> tint_member_7;
};

struct tint_struct_2 {
  const device tint_struct_3* tint_member_6;
  sampler tint_member_13;
  texture2d<float, access::sample> tint_member_14;
  sampler tint_member_15;
  texture2d<float, access::sample> tint_member_16;
  sampler tint_member_17;
  texture2d<float, access::sample> tint_member_18;
  sampler tint_member_19;
  texture2d<float, access::sample> tint_member_20;
  thread uint* tint_member_21;
};

struct tint_struct_5 {
  float4 tint_member_22 [[color(0)]];
};

struct tint_struct_6 {
  uint tint_member_23 [[user(locn0)]] [[flat]];
  float4 tint_member_24 [[user(locn1)]] [[center_no_perspective]];
  float2 tint_member_25 [[user(locn2)]];
  float2 tint_member_26 [[user(locn3)]];
  float tint_member_27 [[user(locn4)]];
};

float4 v(float2 v_1, int v_2, texture2d<float, access::sample> v_3, sampler v_4, texture2d<float, access::sample> v_5, sampler v_6, texture2d<float, access::sample> v_7, sampler v_8, texture2d<float, access::sample> v_9, sampler v_10) {
  switch(v_2) {
    case 1:
    {
      float4 const v_11 = v_5.sample(v_6, v_1, bias(clamp(-0.47499999403953552246f, -16.0f, 15.9899997711181640625f)));
      if (( tint_volatile_zero == 0u)) {
        return v_11;
      }
      break;
    }
    case 2:
    {
      float4 const v_12 = v_7.sample(v_8, v_1, bias(clamp(-0.47499999403953552246f, -16.0f, 15.9899997711181640625f)));
      if (( tint_volatile_zero == 0u)) {
        return v_12;
      }
      break;
    }
    case 3:
    {
      float4 const v_13 = v_9.sample(v_10, v_1, bias(clamp(-0.47499999403953552246f, -16.0f, 15.9899997711181640625f)));
      if (( tint_volatile_zero == 0u)) {
        return v_13;
      }
      break;
    }
    default:
    {
      float4 const v_14 = v_3.sample(v_4, v_1, bias(clamp(-0.47499999403953552246f, -16.0f, 15.9899997711181640625f)));
      if (( tint_volatile_zero == 0u)) {
        return v_14;
      }
      break;
    }
  }
  /* unreachable */
  return 0.0f;
}

int v_15(float v_16) {
  return int(clamp(v_16, -2147483648.0f, 2147483520.0f));
}

void v_17(tint_struct v_18, thread tint_struct_1* const v_19, tint_struct_2 v_20) {
  (*v_20.tint_member_21) = v_18.tint_member;
  float4 const v_21 = v_18.tint_member_1;
  float2 const v_22 = (*v_20.tint_member_6).tint_member_7[(*v_20.tint_member_21)].tint_member_12;
  float4 v_23 = float4(1.0f);
  float4 v_24 = 0.0f;
  float v_25 = (7.96875f * (v(v_18.tint_member_3, v_15(v_18.tint_member_4), v_20.tint_member_14, v_20.tint_member_13, v_20.tint_member_16, v_20.tint_member_15, v_20.tint_member_18, v_20.tint_member_17, v_20.tint_member_20, v_20.tint_member_19).x - 0.50196081399917602539f));
  v_25 = (v_25 - v_22.x);
  float const v_26 = dfdx(v_25);
  float2 v_27 = float2(v_26, dfdy(v_25));
  float const v_28 = dot(v_27, v_27);
  float2 v_29 = 0.0f;
  if ((v_28 >= 0.00009999999747378752f)) {
    v_29 = (v_27 * rsqrt(v_28));
  } else {
    v_29 = float2(0.70709997415542602539f);
  }
  v_27 = v_29;
  float2 const v_30 = dfdx(v_18.tint_member_2);
  float2 const v_31 = dfdy(v_18.tint_member_2);
  float2 const v_32 = float2(v_30.x, v_30.y);
  float2x2 const v_33 = float2x2(v_32, float2(v_31.x, v_31.y));
  float2 const v_34 = float2((v_33 * float2(v_27)));
  float const v_35 = (0.64999997615814208984f * length(v_34));
  if ((v_22.y > 0.0f)) {
    v_24 = float4(saturate(((v_25 + v_35) / (2.0f * v_35))));
  } else {
    float const v_36 = -(v_35);
    float const v_37 = clamp(((v_25 - v_36) / (v_35 - v_36)), 0.0f, 1.0f);
    v_24 = float4((v_37 * (v_37 * (3.0f - (2.0f * v_37)))));
  }
  v_23 = v_24;
  (*v_19).tint_member_5 = (v_21 * v_23);
}

tint_struct_1 v_38(tint_struct v_39, tint_struct_2 v_40) {
  tint_struct_1 v_41 = {};
  (v_17(v_39, (&v_41), v_40));
  return v_41;
}

fragment tint_struct_5 tint_entry_point(tint_struct_6 v_43 [[stage_in]], const device tint_struct_3* v_44 [[buffer(1)]], sampler v_45 [[sampler(0)]], texture2d<float, access::sample> v_46 [[texture(0)]], sampler v_47 [[sampler(1)]], texture2d<float, access::sample> v_48 [[texture(1)]], sampler v_49 [[sampler(2)]], texture2d<float, access::sample> v_50 [[texture(2)]], sampler v_51 [[sampler(3)]], texture2d<float, access::sample> v_52 [[texture(3)]]) {
  thread uint v_53 = 0u;
  tint_struct_2 const v_54 = tint_struct_2{.tint_member_6=v_44, .tint_member_13=v_45, .tint_member_14=v_46, .tint_member_15=v_47, .tint_member_16=v_48, .tint_member_17=v_49, .tint_member_18=v_50, .tint_member_19=v_51, .tint_member_20=v_52, .tint_member_21=(&v_53)};
  tint_struct_5 v_55 = {};
  v_55.tint_member_22 = v_38(tint_struct{.tint_member=v_43.tint_member_23, .tint_member_1=v_43.tint_member_24, .tint_member_2=v_43.tint_member_25, .tint_member_3=v_43.tint_member_26, .tint_member_4=v_43.tint_member_27}, v_54).tint_member_5;
  return v_55;
}
