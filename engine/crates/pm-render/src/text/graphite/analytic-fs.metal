// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
// Dawn/Tint 571752c9cb3ed36a3194854374984427794c4dda; see tools/graphite-oracle/DAWN-LICENSE.
#pragma METAL fp math_mode(relaxed)
#include <metal_stdlib>
using namespace metal;

struct tint_struct {
  float4 tint_member;
  float4 tint_member_1;
  float4 tint_member_2;
  float4 tint_member_3;
  float4 tint_member_4;
  float2 tint_member_5;
  float2 tint_member_6;
};

struct tint_struct_1 {
  float4 tint_member_7;
};

struct tint_struct_3 {
  /* 0x0000 */ float4 tint_member_9;
  /* 0x0010 */ float4 tint_member_10;
  /* 0x0020 */ float4 tint_member_11;
  /* 0x0030 */ float4 tint_member_12;
  /* 0x0040 */ float4 tint_member_13;
  /* 0x0050 */ float4 tint_member_14;
  /* 0x0060 */ float4 tint_member_15;
  /* 0x0070 */ float4 tint_member_16;
  /* 0x0080 */ float4 tint_member_17;
};

struct tint_struct_2 {
  const constant tint_struct_3* tint_member_8;
};

struct tint_struct_4 {
  float4 tint_member_18 [[color(0)]];
};

struct tint_struct_5 {
  float4 tint_member_19 [[user(locn0)]];
  float4 tint_member_20 [[user(locn1)]];
  float4 tint_member_21 [[user(locn2)]];
  float4 tint_member_22 [[user(locn3)]];
  float2 tint_member_23 [[user(locn4)]];
  float2 tint_member_24 [[user(locn5)]];
};

void v(thread float2* const v_1, float2x2 v_2, float2 v_3, float2 v_4, float2 v_5, float2 v_6) {
  float2 const v_7 = (v_6 - v_4);
  if (all((v_7 > float2(0.0f)))) {
    bool v_8 = false;
    if (all((v_6 > float2(0.0f)))) {
      v_8 = true;
    } else {
      bool v_9 = false;
      if ((v_3.x > 0.0f)) {
        v_9 = (v_3.y < 0.0f);
      } else {
        v_9 = false;
      }
      v_8 = v_9;
    }
    if (v_8) {
      float2 const v_10 = (v_7 * v_5);
      float2 const v_11 = (1.0f / ((v_6 * v_6) + (v_3.x * v_3.x)));
      float2 const v_12 = (v_11 * v_10);
      float2 const v_13 = (v_12 * v_2);
      float const v_14 = rsqrt(dot(v_13, v_13));
      float const v_15 = ((0.5f * v_14) * (dot(v_10, v_12) - 1.0f));
      float const v_16 = (((v_6.x * v_3.x) * v_11.x) * v_14);
      float2 v_17 = float2((v_16 - v_15), (v_16 + v_15));
      v_17.y = select(-(v_17.y), 1.0f, ((v_6.x - v_3.x) <= 0.0f));
      (*v_1) = min((*v_1), v_17);
    } else {
      if ((v_3.y == 0.0f)) {
        float2 const v_18 = (v_5 * v_2);
        float const v_19 = (((v_3.x - v_7.x) - v_7.y) * rsqrt(dot(v_18, v_18)));
        (*v_1).x = min((*v_1).x, v_19);
      }
    }
  }
}

void v_20(thread float2* const v_21, float2x2 v_22, float2 v_23, float4 v_24, float4 v_25, float4 v_26) {
  float2 v_27 = (*v_21);
  (v((&v_27), v_22, v_23, v_24.xy, float2(-1.0f), float2(v_25.x, v_26.x)));
  (*v_21) = v_27;
  float2 v_28 = (*v_21);
  (v((&v_28), v_22, v_23, v_24.zy, float2(1.0f, -1.0f), float2(v_25.y, v_26.y)));
  (*v_21) = v_28;
  float2 v_29 = (*v_21);
  (v((&v_29), v_22, v_23, v_24.zw, float2(1.0f), float2(v_25.z, v_26.z)));
  (*v_21) = v_29;
  float2 v_30 = (*v_21);
  (v((&v_30), v_22, v_23, v_24.xw, float2(-1.0f, 1.0f), float2(v_25.w, v_26.w)));
  (*v_21) = v_30;
}

void v_31(tint_struct v_32, thread tint_struct_1* const v_33, tint_struct_2 v_34) {
  float4 v_35 = 0.0f;
  if ((v_32.tint_member_6.x > 0.0f)) {
    v_35 = float4(1.0f);
  } else {
    if ((v_32.tint_member_6.y > 1.0f)) {
      float2 const v_36 = min(v_32.tint_member_2.xy, v_32.tint_member_2.zw);
      float const v_37 = (min(v_36.x, v_36.y) * v_32.tint_member.w);
      float const v_38 = ((v_32.tint_member_6.y - 1.0f) * v_32.tint_member.w);
      float const v_39 = (1.0f - (0.5f * v_38));
      v_35 = float4(float(saturate((v_38 * (v_37 + v_39)))));
    } else {
      float2 const v_40 = float2(v_32.tint_member_1.x, v_32.tint_member_1.y);
      float2x2 const v_41 = (float2x2(v_40, float2(v_32.tint_member_1.z, v_32.tint_member_1.w)) * (1.0f / v_32.tint_member.w));
      float2 const v_42 = (float2(1.0f, 0.0f) * v_41);
      float2 const v_43 = (float2(0.0f, 1.0f) * v_41);
      float2 const v_44 = float2(rsqrt(dot(v_42, v_42)), rsqrt(dot(v_43, v_43)));
      float2 const v_45 = (v_44 * (v_32.tint_member_5.x + min(v_32.tint_member_2.xy, v_32.tint_member_2.zw)));
      float2 v_46 = float2(min(v_45.x, v_45.y), -1.0f);
      float v_47 = 0.0f;
      float v_48 = 0.0f;
      if ((v_32.tint_member_6.x > -0.94999998807907104492f)) {
        float2 const v_49 = (v_44 * ((v_32.tint_member_2.xy + v_32.tint_member_2.zw) + (2.0f * v_32.tint_member_5.xx)));
        v_47 = min(min(v_49.x, v_49.y), 1.0f);
        v_48 = (1.0f - (0.5f * v_47));
      } else {
        float2 const v_50 = ((2.0f * v_32.tint_member_5.x) * v_44);
        float2 const v_51 = (v_50 - v_45);
        v_46.y = -(max(v_51.x, v_51.y));
        if ((v_32.tint_member_5.x > 0.0f)) {
          float const v_52 = min(v_50.x, v_50.y);
          float2 const v_53 = select(float2(v_52), v_50, (v_51 >= float2(-0.5f)));
          v_47 = saturate(max(v_53.x, v_53.y));
          v_48 = (1.0f - (0.5f * v_47));
        } else {
          v_48 = 1.0f;
          v_47 = v_48;
        }
      }
      float2 v_54 = v_46;
      (v_20((&v_54), v_41, v_32.tint_member_5, v_32.tint_member_2, v_32.tint_member_3, v_32.tint_member_4));
      v_46 = v_54;
      float const v_55 = (min(v_32.tint_member_6.y, 0.0f) * v_32.tint_member.w);
      float const v_56 = (v_47 * (min((v_46.x + v_55), -(v_46.y)) + v_48));
      v_35 = float4(float(saturate(v_56)));
    }
  }
  float4 const v_57 = v_35;
  (*v_33).tint_member_7 = (float4((*v_34.tint_member_8).tint_member_17) * v_57);
}

tint_struct_1 v_58(tint_struct v_59, tint_struct_2 v_60) {
  tint_struct_1 v_61 = {};
  (v_31(v_59, (&v_61), v_60));
  return v_61;
}

fragment tint_struct_4 tint_entry_point(float4 v_63 [[position]], tint_struct_5 v_64 [[stage_in]], const constant tint_struct_3* v_65 [[buffer(0)]]) {
  tint_struct_2 const v_66 = tint_struct_2{.tint_member_8=v_65};
  tint_struct_4 v_67 = {};
  v_67.tint_member_18 = v_58(tint_struct{.tint_member=v_63, .tint_member_1=v_64.tint_member_19, .tint_member_2=v_64.tint_member_20, .tint_member_3=v_64.tint_member_21, .tint_member_4=v_64.tint_member_22, .tint_member_5=v_64.tint_member_23, .tint_member_6=v_64.tint_member_24}, v_66).tint_member_7;
  return v_67;
}
