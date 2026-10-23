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

struct tint_struct {
  uint tint_member;
};

struct tint_struct_1 {
  float4 tint_member_1;
  float4 tint_member_2;
  float4 tint_member_3;
  float4 tint_member_4;
  float4 tint_member_5;
  float2 tint_member_6;
  float2 tint_member_7;
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
  float4 tint_member_18 [[position]];
  float4 tint_member_19 [[user(locn0)]];
  float4 tint_member_20 [[user(locn1)]];
  float4 tint_member_21 [[user(locn2)]];
  float4 tint_member_22 [[user(locn3)]];
  float2 tint_member_23 [[user(locn4)]];
  float2 tint_member_24 [[user(locn5)]];
};
const constant tint_array<float2, 9> v = tint_array<float2, 9>{float2(1.0f, 0.0f), float2(1.0f, 0.0f), float2(0.0f, 1.0f), float2(0.0f, 1.0f), float2(1.0f, 0.0f), float2(0.0f, 1.0f), float2(1.0f, 0.0f), float2(0.0f, 1.0f), float2(1.0f, 0.0f)};
const constant tint_array<float2, 9> v_1 = tint_array<float2, 9>{float2(1.0f, 0.0f), float2(0.70710676908493041992f), float2(0.70710676908493041992f), float2(0.0f, 1.0f), float2(0.70710676908493041992f), float2(0.70710676908493041992f), float2(1.0f, 0.0f), float2(0.0f, 1.0f), float2(1.0f, 0.0f)};

float3x3 v_2(float3x3 v_3) {
  float const v_4 = v_3[0].x;
  float const v_5 = v_3[0].y;
  float const v_6 = v_3[0].z;
  float const v_7 = v_3[1].x;
  float const v_8 = v_3[1].y;
  float const v_9 = v_3[1].z;
  float const v_10 = v_3[2].x;
  float const v_11 = v_3[2].y;
  float const v_12 = v_3[2].z;
  float const v_13 = ((v_12 * v_8) - (v_9 * v_11));
  float const v_14 = ((-(v_12) * v_7) + (v_9 * v_10));
  float const v_15 = ((v_11 * v_7) - (v_8 * v_10));
  float const v_16 = (((v_4 * v_13) + (v_5 * v_14)) + (v_6 * v_15));
  float3 const v_17 = float3(v_13, ((-(v_12) * v_5) + (v_6 * v_11)), ((v_9 * v_5) - (v_6 * v_8)));
  float3 const v_18 = float3(v_14, ((v_12 * v_4) - (v_6 * v_10)), ((-(v_9) * v_4) + (v_6 * v_7)));
  return (float3x3(v_17, v_18, float3(v_15, ((-(v_11) * v_4) + (v_5 * v_10)), ((v_8 * v_4) - (v_5 * v_7)))) * (1.0f / v_16));
}

float2 v_19(float2 v_20) {
  return float2(-(v_20.y), v_20.x);
}

uint v_21(uint v_22, uint v_23) {
  uint const v_24 = select(v_23, 1u, (v_23 == 0u));
  return (v_22 - ((v_22 / v_24) * v_24));
}

float4 v_25(uint v_26, float2 v_27, float2 v_28, float v_29, float v_30, float4 v_31, float4 v_32, float4 v_33, float4 v_34, float v_35, float3x3 v_36, thread float4* const v_37, thread float4* const v_38, thread float4* const v_39, thread float4* const v_40, thread float2* const v_41, thread float2* const v_42, thread float2* const v_43) {
  float v_44 = 1.0f;
  uint const v_45 = v_21((v_26 + 1u), 4u);
  bool const v_46 = (v_34.z <= 0.0f);
  bool v_47 = false;
  float4 v_48 = 0.0f;
  float4 v_49 = 0.0f;
  float4 v_50 = float4(1.0f);
  bool v_51 = false;
  if ((v_31.x < -1.0f)) {
    v_51 = (v_31.y > 0.0f);
    v_48 = select(v_33.xzzx, v_33.xxzz, bool4(v_51));
    v_49 = v_33.yyww;
    if ((v_31.y < 0.0f)) {
      (*v_39) = (-(v_31) - 2.0f);
      (*v_40) = v_32;
      (*v_41) = float2(0.0f, 1.0f);
    } else {
      (*v_39) = v_32;
      (*v_40) = (*v_39);
      (*v_41) = v_31.zw;
      float v_52 = 0.0f;
      if (((*v_41).y < 0.0f)) {
        v_52 = 0.41421356797218322754f;
      } else {
        v_52 = sign((*v_41).y);
      }
      v_44 = v_52;
    }
  } else {
    if (any((v_31 > float4(0.0f)))) {
      v_48 = v_33.xzzx;
      v_49 = v_33.yyww;
      (*v_39) = v_31;
      (*v_40) = v_32;
      (*v_41) = float2(0.0f, -1.0f);
    } else {
      v_48 = v_32;
      v_49 = v_33;
      v_50 = -(v_31);
      (*v_39) = float4(0.0f);
      (*v_40) = float4(0.0f);
      (*v_41) = float2(0.0f, 1.0f);
      v_47 = true;
    }
  }
  float2 v_53 = float2((*v_39)[v_26], (*v_40)[v_26]);
  if ((v_21(v_26, 2u) != 0u)) {
    v_53 = v_53.yx;
  }
  float2 v_54 = float2(1.0f);
  if (all((v_53 > float2(0.0f)))) {
    v_44 = 0.41421356797218322754f;
    v_54 = v_53.yx;
  }
  float4 v_55 = (v_48 - v_48.wxyz);
  float4 v_56 = (v_49 - v_49.wxyz);
  float4 const v_57 = (1.0f / max(abs(v_55), max(abs(v_56), float4(1.0f))));
  v_55 = (v_55 * v_57);
  v_56 = (v_56 * v_57);
  float4 v_58 = ((v_55 * v_55) + (v_56 * v_56));
  float4 const v_59 = sign(v_58);
  float4 v_60 = float4(0.0f);
  float2 v_61 = float2((*v_41).x);
  if (any((v_59 == float4(0.0f)))) {
    if (all((v_59 == float4(0.0f)))) {
      v_55 = float4(0.0f, 1.0f, 0.0f, -1.0f);
      v_56 = float4(-1.0f, 0.0f, 1.0f, 0.0f);
      v_58 = float4(1.0f);
    } else {
      bool const v_62 = ((((v_59.x + v_59.y) + v_59.z) + v_59.w) > 2.5f);
      float4 const v_63 = v_56.yzwx;
      float4 const v_64 = v_55.yzwx;
      float4 const v_65 = select(v_63, v_64, bool4(v_62));
      float4 const v_66 = -(v_55.yzwx);
      float4 const v_67 = v_56.yzwx;
      float4 const v_68 = select(v_66, v_67, bool4(v_62));
      v_55 = mix(v_65, v_55, v_59);
      v_56 = mix(v_68, v_56, v_59);
      v_58 = mix(v_58.yzwx, v_58, v_59);
      v_50 = mix(v_50.yzwx, v_50, v_59);
      bool v_69 = false;
      if (!(v_62)) {
        v_69 = (v_44 == 0.0f);
      } else {
        v_69 = false;
      }
      if (v_69) {
        float2 const v_70 = v_61;
        v_61 = (v_70 * float2(v_59[v_26], v_59[v_45]));
        v_60 = ((v_59 - 1.0f) * (*v_41).x);
        (*v_41).y = 1.0f;
        v_44 = 1.0f;
      }
    }
  }
  float4 const v_71 = rsqrt(v_58);
  v_55 = (v_55 * v_71);
  v_56 = (v_56 * v_71);
  float2 const v_72 = -(float2(v_55[v_45], v_56[v_45]));
  float2 const v_73 = float2(v_55[v_26], v_56[v_26]);
  float2 v_74 = 0.0f;
  bool v_75 = false;
  if ((v_29 < 0.0f)) {
    bool v_76 = false;
    if ((v_34.w < 0.0f)) {
      v_76 = true;
    } else {
      v_76 = ((v_30 * v_34.z) != 0.0f);
    }
    if (v_76) {
      v_75 = true;
    } else {
      float const v_77 = v_34.w;
      float2 const v_78 = v_53;
      float2 const v_79 = v_61;
      float2 const v_80 = -(v_61);
      float2 const v_81 = (v_78 + select(v_79, v_80, bool2(v_46)));
      bool v_82 = false;
      if ((v_44 == 1.0f)) {
        v_82 = true;
      } else {
        v_82 = any((v_81 <= float2(v_77)));
      }
      if (v_82) {
        v_74 = (v_81 - v_77);
      } else {
        v_74 = ((v_81 * v_27) - (v_77 * v_28));
      }
    }
  } else {
    v_74 = ((v_53 + v_61) * (v_27 + (v_44 * v_27.yx)));
  }
  if (v_75) {
    v_74 = v_34.xy;
  } else {
    v_74 = (v_74 - v_53);
    float2 const v_83 = float2(v_48[v_26], v_49[v_26]);
    v_74 = ((v_83 + (v_72 * v_74.x)) + (v_73 * v_74.y));
  }
  (*v_38) = (((v_56 * (v_48 - v_74.x)) - (v_55 * (v_49 - v_74.y))) + v_60);
  float3x3 const v_84 = v_2(v_36);
  float3 v_85 = (v_36 * float3(v_74, 1.0f));
  (*v_37) = float4((v_84[0].xy - (v_84[0].z * v_74)), (v_84[1].xy - (v_84[1].z * v_74)));
  if (v_47) {
    float4 const v_86 = ((-(v_56) * (v_84[0].x - (v_84[0].z * v_48))) + (v_55 * (v_84[0].y - (v_84[0].z * v_49))));
    float4 const v_87 = ((-(v_56) * (v_84[1].x - (v_84[1].z * v_48))) + (v_55 * (v_84[1].y - (v_84[1].z * v_49))));
    (*v_38) = ((*v_38) * rsqrt(((v_86 * v_86) + (v_87 * v_87))));
    (*v_38) = ((*v_38) + ((1.0f - v_50) * abs(v_85.z)));
    bool v_88 = false;
    if (all((v_50 == float4(1.0f)))) {
      v_88 = (dot(abs(((v_55 * v_55.yzwx) + (v_56 * v_56.yzwx))), float4(1.0f)) < 0.00023999999393709004f);
    } else {
      v_88 = false;
    }
    bool const v_89 = v_88;
    if (v_89) {
      float2 const v_90 = ((*v_38).xy + (*v_38).zw);
      (*v_42).y = (1.0f + min(min(v_90.x, v_90.y), abs(v_85.z)));
    } else {
      (*v_42).y = (1.0f + abs(v_85.z));
    }
  }
  bool v_91 = false;
  if ((v_29 > 0.0f)) {
    v_91 = (v_85.z > 0.0f);
  } else {
    v_91 = false;
  }
  if (v_91) {
    float const v_92 = (*v_37).z;
    float const v_93 = (*v_37).w;
    float2 const v_94 = float2((*v_37).x, (*v_37).y);
    float2x2 const v_95 = float2x2(v_94, float2(v_92, v_93));
    float2 const v_96 = (float2(v_50[v_26], v_50[v_45]) * v_28);
    float const v_97 = (v_54.x * v_96.x);
    float2 v_98 = ((v_97 * v_19(-(v_73))) * v_95);
    float const v_99 = (v_54.y * v_96.y);
    float2 v_100 = ((v_99 * v_19(v_72)) * v_95);
    bool const v_101 = all((v_96 != float2(0.0f)));
    bool v_102 = false;
    if ((v_44 == 1.0f)) {
      v_102 = v_101;
    } else {
      v_102 = false;
    }
    if (v_102) {
      v_98 = normalize(v_98);
      v_100 = normalize(v_100);
      if ((dot(v_98, v_100) < -0.80000001192092895508f)) {
        float const v_103 = v_100.x;
        float const v_104 = v_100.y;
        float2 const v_105 = float2(v_98.x, v_98.y);
        float const v_106 = sign(determinant(float2x2(v_105, float2(v_103, v_104))));
        v_98 = (v_106 * v_19(v_98));
        v_100 = (-(v_106) * v_19(v_100));
      }
    }
    v_85 = float3((v_85.xy + (v_85.z * normalize((v_98 + v_100)))), v_85.z);
    if (v_47) {
      (*v_38) = ((*v_38) - v_85.z);
    } else {
      (*v_42).y = -(v_85.z);
    }
  } else {
    if (!(v_47)) {
      (*v_42).y = 0.0f;
    }
  }
  float v_107 = 0.0f;
  if ((v_30 != 0.0f)) {
    v_107 = 1.0f;
  } else {
    v_107 = select(0.0f, -1.0f, v_46);
  }
  (*v_42).x = float(v_107);
  if (v_51) {
    float const v_108 = -(v_55.x);
    float const v_109 = v_55.y;
    float2 const v_110 = float2(v_56.x, -(v_56.y));
    float2x2 const v_111 = float2x2(v_110, float2(v_108, v_109));
    float const v_112 = (*v_37).z;
    float const v_113 = (*v_37).w;
    float2 const v_114 = float2((*v_37).x, (*v_37).y);
    float2x2 const v_115 = (v_111 * float2x2(v_114, float2(v_112, v_113)));
    (*v_37) = float4(v_115[0], v_115[1]);
  }
  (*v_43) = v_74;
  return float4(v_85.xy, (v_85.z * v_35), v_85.z);
}

uint v_116(uint v_117, uint v_118) {
  return (v_117 / select(v_118, 1u, (v_118 == 0u)));
}

void v_119(tint_struct v_120, thread tint_struct_1* const v_121, tint_struct_2 v_122) {
  uint const v_123 = v_116(uint(int(v_120.tint_member)), 9u);
  uint const v_124 = v_21(uint(int(v_120.tint_member)), 9u);
  float v_125 = 0.0f;
  if ((v_124 < 4u)) {
    v_125 = 1.0f;
  } else {
    v_125 = select(-1.0f, 0.0f, (v_124 < 6u));
  }
  float const v_126 = float(v_125);
  float const v_127 = float((v_124 == 8u));
  float2 v_128 = 0.0f;
  float4 v_129 = 0.0f;
  float4 v_130 = 0.0f;
  float4 v_131 = 0.0f;
  float4 v_132 = 0.0f;
  float2 v_133 = 0.0f;
  float2 v_134 = 0.0f;
  float2 v_135 = 0.0f;
  float3 const v_136 = (*v_122.tint_member_8).tint_member_14.xyz;
  float3 const v_137 = (*v_122.tint_member_8).tint_member_15.xyz;
  float3 const v_138 = (*v_122.tint_member_8).tint_member_16.xyz;
  float4 const v_139 = (*v_122.tint_member_8).tint_member_10;
  float4 const v_140 = (*v_122.tint_member_8).tint_member_11;
  float4 const v_141 = (*v_122.tint_member_8).tint_member_12;
  float4 const v_142 = (*v_122.tint_member_8).tint_member_13;
  float3 const v_143 = float3(v_136.x, v_136.y, v_136.z);
  float3 const v_144 = float3(v_137.x, v_137.y, v_137.z);
  float4 const v_145 = v_25(v_123, v[v_124], v_1[v_124], v_126, v_127, v_139, v_140, v_141, v_142, 0.5f, float3x3(v_143, v_144, float3(v_138.x, v_138.y, v_138.z)), (&v_129), (&v_130), (&v_131), (&v_132), (&v_133), (&v_134), (&v_135));
  (*v_121).tint_member_2 = v_129;
  (*v_121).tint_member_3 = v_130;
  (*v_121).tint_member_4 = v_131;
  (*v_121).tint_member_5 = v_132;
  (*v_121).tint_member_6 = v_133;
  (*v_121).tint_member_7 = v_134;
  v_128 = v_135;
  float4 const v_146 = v_145;
  (*v_121).tint_member_1 = float4((((*v_122.tint_member_8).tint_member_9.zw * v_146.xy) - (sign((*v_122.tint_member_8).tint_member_9.zw) * v_146.ww)), v_146.zw);
}

tint_struct_1 v_147(tint_struct v_148, tint_struct_2 v_149) {
  tint_struct_1 v_150 = {};
  (v_119(v_148, (&v_150), v_149));
  return v_150;
}

vertex tint_struct_4 tint_entry_point(uint v_152 [[vertex_id]], const constant tint_struct_3* v_153 [[buffer(0)]]) {
  tint_struct_2 const v_154 = tint_struct_2{.tint_member_8=v_153};
  tint_struct_1 const v_155 = v_147(tint_struct{.tint_member=v_152}, v_154);
  tint_struct_4 v_156 = {};
  v_156.tint_member_18 = v_155.tint_member_1;
  v_156.tint_member_19 = v_155.tint_member_2;
  v_156.tint_member_20 = v_155.tint_member_3;
  v_156.tint_member_21 = v_155.tint_member_4;
  v_156.tint_member_22 = v_155.tint_member_5;
  v_156.tint_member_23 = v_155.tint_member_6;
  v_156.tint_member_24 = v_155.tint_member_7;
  return v_156;
}
