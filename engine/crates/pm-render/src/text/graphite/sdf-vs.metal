// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
// Dawn/Tint 571752c9cb3ed36a3194854374984427794c4dda; see tools/graphite-oracle/DAWN-LICENSE.
#pragma METAL fp math_mode(relaxed)
#include <metal_stdlib>
using namespace metal;

struct tint_struct {
  uint tint_member;
  uint2 tint_member_1;
  uint2 tint_member_2;
  float2 tint_member_3;
  uint2 tint_member_4;
  float tint_member_5;
  float tint_member_6;
  uint tint_member_7;
};

struct tint_struct_1 {
  float4 tint_member_8;
  uint tint_member_9;
  float4 tint_member_10;
  float2 tint_member_11;
  float2 tint_member_12;
  float tint_member_13;
};

struct tint_struct_3 {
  /* 0x0000 */ float4 tint_member_15;
  /* 0x0010 */ float4 tint_member_16;
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

struct tint_struct_5 {
  /* 0x0000 */ float4 tint_member_19;
  /* 0x0010 */ float4x4 tint_member_20;
  /* 0x0050 */ float4x4 tint_member_21;
  /* 0x0090 */ float2 tint_member_22;
  /* 0x0098 */ float2 tint_member_23;
};

struct tint_struct_4 {
  /* 0x0000 */ tint_array<tint_struct_5, 1> tint_member_18;
};

struct tint_struct_2 {
  const constant tint_struct_3* tint_member_14;
  const device tint_struct_4* tint_member_17;
  thread uint* tint_member_24;
};

struct tint_struct_6 {
  float4 tint_member_25 [[position]];
  uint tint_member_26 [[user(locn0)]] [[flat]];
  float4 tint_member_27 [[user(locn1)]] [[center_no_perspective]];
  float2 tint_member_28 [[user(locn2)]];
  float2 tint_member_29 [[user(locn3)]];
  float tint_member_30 [[user(locn4)]];
};

struct tint_struct_7 {
  uint2 tint_member_31 [[attribute(0)]];
  uint2 tint_member_32 [[attribute(1)]];
  float2 tint_member_33 [[attribute(2)]];
  uint2 tint_member_34 [[attribute(3)]];
  float tint_member_35 [[attribute(4)]];
  float tint_member_36 [[attribute(5)]];
  uint tint_member_37 [[attribute(6)]];
};

float4x4 v(float4x4 v_1) {
  float const v_2 = v_1[0].x;
  float const v_3 = v_1[0].y;
  float const v_4 = v_1[0].z;
  float const v_5 = v_1[0].w;
  float const v_6 = v_1[1].x;
  float const v_7 = v_1[1].y;
  float const v_8 = v_1[1].z;
  float const v_9 = v_1[1].w;
  float const v_10 = v_1[2].x;
  float const v_11 = v_1[2].y;
  float const v_12 = v_1[2].z;
  float const v_13 = v_1[2].w;
  float const v_14 = v_1[3].x;
  float const v_15 = v_1[3].y;
  float const v_16 = v_1[3].z;
  float const v_17 = v_1[3].w;
  float const v_18 = ((v_2 * v_7) - (v_3 * v_6));
  float const v_19 = ((v_2 * v_8) - (v_4 * v_6));
  float const v_20 = ((v_2 * v_9) - (v_5 * v_6));
  float const v_21 = ((v_3 * v_8) - (v_4 * v_7));
  float const v_22 = ((v_3 * v_9) - (v_5 * v_7));
  float const v_23 = ((v_4 * v_9) - (v_5 * v_8));
  float const v_24 = ((v_10 * v_15) - (v_11 * v_14));
  float const v_25 = ((v_10 * v_16) - (v_12 * v_14));
  float const v_26 = ((v_10 * v_17) - (v_13 * v_14));
  float const v_27 = ((v_11 * v_16) - (v_12 * v_15));
  float const v_28 = ((v_11 * v_17) - (v_13 * v_15));
  float const v_29 = ((v_12 * v_17) - (v_13 * v_16));
  float const v_30 = ((((((v_18 * v_29) - (v_19 * v_28)) + (v_20 * v_27)) + (v_21 * v_26)) - (v_22 * v_25)) + (v_23 * v_24));
  float4 const v_31 = float4((((v_7 * v_29) - (v_8 * v_28)) + (v_9 * v_27)), (((v_4 * v_28) - (v_3 * v_29)) - (v_5 * v_27)), (((v_15 * v_23) - (v_16 * v_22)) + (v_17 * v_21)), (((v_12 * v_22) - (v_11 * v_23)) - (v_13 * v_21)));
  float4 const v_32 = float4((((v_8 * v_26) - (v_6 * v_29)) - (v_9 * v_25)), (((v_2 * v_29) - (v_4 * v_26)) + (v_5 * v_25)), (((v_16 * v_20) - (v_14 * v_23)) - (v_17 * v_19)), (((v_10 * v_23) - (v_12 * v_20)) + (v_13 * v_19)));
  float4 const v_33 = float4((((v_6 * v_28) - (v_7 * v_26)) + (v_9 * v_24)), (((v_3 * v_26) - (v_2 * v_28)) - (v_5 * v_24)), (((v_14 * v_22) - (v_15 * v_20)) + (v_17 * v_18)), (((v_11 * v_20) - (v_10 * v_22)) - (v_13 * v_18)));
  return (float4x4(v_31, v_32, v_33, float4((((v_7 * v_25) - (v_6 * v_27)) - (v_8 * v_24)), (((v_2 * v_27) - (v_3 * v_25)) + (v_4 * v_24)), (((v_15 * v_19) - (v_14 * v_21)) - (v_16 * v_18)), (((v_10 * v_21) - (v_11 * v_19)) + (v_12 * v_18)))) * (1.0f / v_30));
}

float4 v_34(float2 v_35, float4x4 v_36, float4x4 v_37, float2 v_38, float2 v_39, float2 v_40, float2 v_41, float v_42, float v_43, thread float2* const v_44, thread float2* const v_45, thread float2* const v_46) {
  float2 v_47 = v_35;
  v_47 = (v_47 * v_39);
  float2 const v_48 = ((v_42 * v_47) + v_41);
  float4 const v_49 = (v_36 * float4(v_48, 0.0f, 1.0f));
  (*v_46) = (v(v_37) * v_49).xy;
  (*v_45) = (v_47 + v_40);
  (*v_44) = ((*v_45) * v_38);
  return float4(v_49.xy, (v_43 * v_49.w), v_49.w);
}

void v_50(tint_struct v_51, thread tint_struct_1* const v_52, tint_struct_2 v_53) {
  float2 v_54 = float2(0.0f);
  (*v_53.tint_member_24) = v_51.tint_member_7;
  float4x4 const v_55 = (*v_53.tint_member_17).tint_member_18[(*v_53.tint_member_24)].tint_member_20;
  float4x4 const v_56 = (*v_53.tint_member_17).tint_member_18[(*v_53.tint_member_24)].tint_member_21;
  float2 const v_57 = (*v_53.tint_member_17).tint_member_18[(*v_53.tint_member_24)].tint_member_22;
  (*v_52).tint_member_13 = float(float(v_51.tint_member_4.x));
  float2 v_58 = 0.0f;
  float2 v_59 = 0.0f;
  float2 v_60 = 0.0f;
  float const v_61 = float((int(v_51.tint_member) >> (1u & 31u)));
  float2 const v_62 = float2(v_61, float((int(v_51.tint_member) & 1)));
  float2 const v_63 = float2(v_51.tint_member_1);
  float4 const v_64 = v_34(v_62, v_55, v_56, v_57, v_63, float2(v_51.tint_member_2), v_51.tint_member_3, v_51.tint_member_5, v_51.tint_member_6, (&v_58), (&v_59), (&v_60));
  (*v_52).tint_member_12 = v_58;
  (*v_52).tint_member_11 = v_59;
  v_54 = v_60;
  float4 const v_65 = v_64;
  (*v_52).tint_member_8 = float4((((*v_53.tint_member_14).tint_member_15.zw * v_65.xy) - (sign((*v_53.tint_member_14).tint_member_15.zw) * v_65.ww)), v_65.zw);
  (*v_52).tint_member_10 = (*v_53.tint_member_17).tint_member_18[(*v_53.tint_member_24)].tint_member_19;
  (*v_52).tint_member_9 = (*v_53.tint_member_24);
}

tint_struct_1 v_66(tint_struct v_67, tint_struct_2 v_68) {
  tint_struct_1 v_69 = {};
  (v_50(v_67, (&v_69), v_68));
  return v_69;
}

vertex tint_struct_6 tint_entry_point(uint v_71 [[vertex_id]], tint_struct_7 v_72 [[stage_in]], const constant tint_struct_3* v_73 [[buffer(0)]], const device tint_struct_4* v_74 [[buffer(1)]]) {
  thread uint v_75 = 0u;
  tint_struct_2 const v_76 = tint_struct_2{.tint_member_14=v_73, .tint_member_17=v_74, .tint_member_24=(&v_75)};
  tint_struct_1 const v_77 = v_66(tint_struct{.tint_member=v_71, .tint_member_1=v_72.tint_member_31, .tint_member_2=v_72.tint_member_32, .tint_member_3=v_72.tint_member_33, .tint_member_4=v_72.tint_member_34, .tint_member_5=v_72.tint_member_35, .tint_member_6=v_72.tint_member_36, .tint_member_7=v_72.tint_member_37}, v_76);
  tint_struct_6 v_78 = {};
  v_78.tint_member_25 = v_77.tint_member_8;
  v_78.tint_member_26 = v_77.tint_member_9;
  v_78.tint_member_27 = v_77.tint_member_10;
  v_78.tint_member_28 = v_77.tint_member_11;
  v_78.tint_member_29 = v_77.tint_member_12;
  v_78.tint_member_30 = v_77.tint_member_13;
  return v_78;
}
