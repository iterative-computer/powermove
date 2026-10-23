// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
// Dawn/Tint 571752c9cb3ed36a3194854374984427794c4dda; see tools/graphite-oracle/DAWN-LICENSE.
#pragma METAL fp math_mode(relaxed)
#include <metal_stdlib>
using namespace metal;

struct tint_struct {
  float2 tint_member;
  float4 tint_member_1;
  float4 tint_member_2;
  float2 tint_member_3;
  float tint_member_4;
  uint tint_member_5;
  float tint_member_6;
};

struct tint_struct_1 {
  float4 tint_member_7;
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

struct tint_struct_3 {
  /* 0x0000 */ tint_array<float4, 12> tint_member_9;
};

struct tint_struct_2 {
  const constant tint_struct_3* tint_member_8;
};

struct tint_struct_4 {
  float4 tint_member_10 [[position]];
};

struct tint_struct_5 {
  float2 tint_member_11 [[attribute(0)]];
  float4 tint_member_12 [[attribute(1)]];
  float4 tint_member_13 [[attribute(2)]];
  float2 tint_member_14 [[attribute(3)]];
  float tint_member_15 [[attribute(4)]];
  uint tint_member_16 [[attribute(5)]];
  float tint_member_17 [[attribute(6)]];
};

float v(float2 v_1, float2 v_2, float2 v_3, float2 v_4, float2x2 v_5) {
  float2 const v_6 = (v_5 * (fma(float2(-2.0f), v_2, v_3) + v_1));
  float2 const v_7 = (v_5 * (fma(float2(-2.0f), v_3, v_4) + v_2));
  return max(dot(v_6, v_6), dot(v_7, v_7));
}

float v_8(float2 v_9, float2 v_10, float2 v_11, float v_12) {
  float2 v_13 = v_9;
  float2 v_14 = v_10;
  float2 v_15 = v_11;
  float2 const v_16 = ((min(min(v_13, v_14), v_15) + max(max(v_13, v_14), v_15)) * 0.5f);
  v_13 = (v_13 - v_16);
  v_14 = (v_14 - v_16);
  v_15 = (v_15 - v_16);
  float const v_17 = sqrt(max(max(dot(v_13, v_13), dot(v_14, v_14)), dot(v_15, v_15)));
  float2 const v_18 = float2((-2.0f * v_12));
  float2 const v_19 = (fma(v_18, v_14, v_13) + v_15);
  float const v_20 = abs(fma(-2.0f, v_12, 2.0f));
  float const v_21 = max(0.0f, fma(v_17, 4.0f, -1.0f));
  float const v_22 = ((length(v_19) * 4.0f) + (v_21 * v_20));
  float const v_23 = (4.0f * min(v_12, 1.0f));
  return (v_22 / v_23);
}

int v_24(float v_25) {
  return int(clamp(v_25, -2147483648.0f, 2147483520.0f));
}

void v_26(tint_struct v_27, thread tint_struct_1* const v_28, tint_struct_2 v_29) {
  float4 const v_30 = (*v_29.tint_member_8).tint_member_9[1];
  float4 const v_31 = (*v_29.tint_member_8).tint_member_9[2];
  float4 const v_32 = (*v_29.tint_member_8).tint_member_9[3];
  float4 const v_33 = (*v_29.tint_member_8).tint_member_9[4];
  float4 const v_34 = float4(v_30.x, v_30.y, v_30.z, v_30.w);
  float4 const v_35 = float4(v_31.x, v_31.y, v_31.z, v_31.w);
  float4 const v_36 = float4(v_32.x, v_32.y, v_32.z, v_32.w);
  float4x4 const v_37 = float4x4(v_34, v_35, v_36, float4(v_33.x, v_33.y, v_33.z, v_33.w));
  float2 v_38 = 0.0f;
  if ((v_27.tint_member.x < 0.0f)) {
    v_38 = v_27.tint_member_3;
  } else {
    float2 const v_39 = v_37[0].xy;
    float2 const v_40 = v_37[1].xy;
    float2 const v_41 = float2(v_39.x, v_39.y);
    float2x2 const v_42 = float2x2(v_41, float2(v_40.x, v_40.y));
    float v_43 = v_27.tint_member.x;
    float v_44 = v_27.tint_member.y;
    float2 v_45 = 0.0f;
    if ((v_27.tint_member_6 == 2.0f)) {
      float2 v_46 = 0.0f;
      if ((v_43 != 0.0f)) {
        v_46 = v_27.tint_member_1.zw;
      } else {
        v_46 = select(v_27.tint_member_1.xy, v_27.tint_member_2.xy, bool2((v_44 != 0.0f)));
      }
      v_45 = v_46;
    } else {
      float2 const v_47 = v_27.tint_member_1.xy;
      float2 v_48 = v_27.tint_member_1.zw;
      float2 const v_49 = v_27.tint_member_2.xy;
      float2 v_50 = v_27.tint_member_2.zw;
      float v_51 = -1.0f;
      float v_52 = 0.0f;
      if ((v_27.tint_member_6 != 0.0f)) {
        v_51 = v_50.x;
        float const v_53 = v_8((v_42 * v_47), (v_42 * v_48), (v_42 * v_49), v_51);
        v_52 = ceil((log2(max(v_53, 1.0f)) * 0.5f));
        v_48 = (v_48 * v_51);
        v_50 = v_49;
      } else {
        float const v_54 = v(v_47, v_48, v_49, v_50, v_42);
        v_52 = ceil((log2(max((9.0f * v_54), 1.0f)) * 0.25f));
      }
      if ((v_43 > v_52)) {
        float const v_55 = v_44;
        v_44 = floor(ldexp(v_55, v_24((v_52 - v_43))));
        v_43 = v_52;
      }
      float const v_56 = v_44;
      float const v_57 = floor((0.5f + ldexp(v_56, v_24((5.0f - v_43)))));
      bool v_58 = false;
      if ((0.0f < v_57)) {
        v_58 = (v_57 < 32.0f);
      } else {
        v_58 = false;
      }
      if (v_58) {
        float const v_59 = (v_57 * 0.03125f);
        float2 const v_60 = v_48;
        float2 const v_61 = mix(v_47, v_60, float2(v_59));
        float2 const v_62 = v_48;
        float2 const v_63 = mix(v_62, v_49, float2(v_59));
        float2 const v_64 = v_50;
        float2 const v_65 = mix(v_49, v_64, float2(v_59));
        float2 const v_66 = mix(v_61, v_63, float2(v_59));
        float2 const v_67 = mix(v_63, v_65, float2(v_59));
        float2 const v_68 = mix(v_66, v_67, float2(v_59));
        float const v_69 = mix(1.0f, v_51, v_59);
        float const v_70 = ((v_51 + 1.0f) - v_69);
        float const v_71 = mix(v_69, v_70, v_59);
        float2 v_72 = 0.0f;
        if ((v_51 < 0.0f)) {
          v_72 = v_68;
        } else {
          v_72 = (v_66 / v_71);
        }
        v_45 = v_72;
      } else {
        float2 const v_73 = v_50;
        v_45 = select(v_73, v_47, bool2((v_57 == 0.0f)));
      }
    }
    v_38 = v_45;
  }
  float4 v_74 = (v_37 * float4(v_38, 0.0f, 1.0f));
  v_74.z = v_27.tint_member_4;
  (*v_28).tint_member_7 = float4((((*v_29.tint_member_8).tint_member_9[0].zw * v_74.xy) - (sign((*v_29.tint_member_8).tint_member_9[0].zw) * v_74.ww)), v_74.zw);
}

tint_struct_1 v_75(tint_struct v_76, tint_struct_2 v_77) {
  tint_struct_1 v_78 = {};
  (v_26(v_76, (&v_78), v_77));
  return v_78;
}

vertex tint_struct_4 tint_entry_point(tint_struct_5 v_80 [[stage_in]], const constant tint_struct_3* v_81 [[buffer(0)]]) {
  tint_struct_2 const v_82 = tint_struct_2{.tint_member_8=v_81};
  tint_struct_4 v_83 = {};
  v_83.tint_member_10 = v_75(tint_struct{.tint_member=v_80.tint_member_11, .tint_member_1=v_80.tint_member_12, .tint_member_2=v_80.tint_member_13, .tint_member_3=v_80.tint_member_14, .tint_member_4=v_80.tint_member_15, .tint_member_5=v_80.tint_member_16, .tint_member_6=v_80.tint_member_17}, v_82).tint_member_7;
  return v_83;
}
