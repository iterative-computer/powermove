// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
// Dawn/Tint 571752c9cb3ed36a3194854374984427794c4dda; see tools/graphite-oracle/DAWN-LICENSE.
#pragma METAL fp math_mode(relaxed)
#include <metal_stdlib>
using namespace metal;

struct tint_struct {
  uint tint_member;
  float4 tint_member_1;
  float4 tint_member_2;
  float2 tint_member_3;
  float2 tint_member_4;
  float tint_member_5;
  uint tint_member_6;
  float tint_member_7;
};

struct tint_struct_1 {
  float4 tint_member_8;
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
  /* 0x0000 */ tint_array<float4, 12> tint_member_10;
};

struct tint_struct_2 {
  const constant tint_struct_3* tint_member_9;
};

struct tint_struct_4 {
  float4 tint_member_11 [[position]];
};

struct tint_struct_5 {
  float4 tint_member_12 [[attribute(0)]];
  float4 tint_member_13 [[attribute(1)]];
  float2 tint_member_14 [[attribute(2)]];
  float2 tint_member_15 [[attribute(3)]];
  float tint_member_16 [[attribute(4)]];
  uint tint_member_17 [[attribute(5)]];
  float tint_member_18 [[attribute(6)]];
};

float2x2 v(float2x2 v_1) {
  float2 const v_2 = float2(v_1[1].y, -(v_1[0].y));
  return (float2x2(v_2, float2(-(v_1[1].x), v_1[0].x)) * (1.0f / determinant(v_1)));
}

float v_3(float2 v_4, float2 v_5, float2 v_6, float2 v_7, float2x2 v_8) {
  float2 const v_9 = (v_8 * (fma(float2(-2.0f), v_5, v_6) + v_4));
  float2 const v_10 = (v_8 * (fma(float2(-2.0f), v_6, v_7) + v_5));
  return max(dot(v_9, v_9), dot(v_10, v_10));
}

float v_11(float2 v_12, float2 v_13, float2 v_14, float v_15) {
  float2 v_16 = v_12;
  float2 v_17 = v_13;
  float2 v_18 = v_14;
  float2 const v_19 = ((min(min(v_16, v_17), v_18) + max(max(v_16, v_17), v_18)) * 0.5f);
  v_16 = (v_16 - v_19);
  v_17 = (v_17 - v_19);
  v_18 = (v_18 - v_19);
  float const v_20 = sqrt(max(max(dot(v_16, v_16), dot(v_17, v_17)), dot(v_18, v_18)));
  float2 const v_21 = float2((-2.0f * v_15));
  float2 const v_22 = (fma(v_21, v_17, v_16) + v_18);
  float const v_23 = abs(fma(-2.0f, v_15, 2.0f));
  float const v_24 = max(0.0f, fma(v_20, 4.0f, -1.0f));
  float const v_25 = ((length(v_22) * 4.0f) + (v_24 * v_23));
  float const v_26 = (4.0f * min(v_15, 1.0f));
  return (v_25 / v_26);
}

float2 v_27(float2 v_28, float2 v_29) {
  float2 const v_30 = (v_28 - v_29);
  if (all((v_30 == float2(0.0f)))) {
    return float2(0.0f);
  } else {
    float const v_31 = (1.0f / max(abs(v_30.x), abs(v_30.y)));
    return normalize((v_31 * v_30));
  }
  /* unreachable */
  return 0.0f;
}

float2 v_32(float2 v_33, float2 v_34, float v_35) {
  return fma((v_34 - v_33), float2(v_35), v_33);
}

void v_36(tint_struct v_37, thread tint_struct_1* const v_38, tint_struct_2 v_39) {
  float v_40 = float((int(v_37.tint_member) >> (1u & 31u)));
  if (((int(v_37.tint_member) & 1) != 0)) {
    v_40 = -(v_40);
  }
  float4 v_41 = 0.0f;
  float2 const v_42 = (*v_39.tint_member_9).tint_member_10[1].xy;
  float2 const v_43 = (*v_39.tint_member_9).tint_member_10[1].zw;
  float2 const v_44 = float2(v_42.x, v_42.y);
  float2x2 const v_45 = float2x2(v_44, float2(v_43.x, v_43.y));
  float2 v_46 = v_37.tint_member_3;
  float2 v_47 = v_37.tint_member_1.xy;
  float2 v_48 = v_37.tint_member_1.zw;
  float2 v_49 = v_37.tint_member_2.xy;
  float2 v_50 = v_37.tint_member_2.zw;
  float v_51 = -1.0f;
  if ((v_37.tint_member_7 != 0.0f)) {
    v_51 = v_50.x;
    v_50 = v_49;
  }
  float v_52 = 0.0f;
  if ((v_51 < 0.0f)) {
    bool v_53 = false;
    if (all((v_47 == v_48))) {
      v_53 = all((v_49 == v_50));
    } else {
      v_53 = false;
    }
    if (v_53) {
      v_52 = 1.0f;
    } else {
      float const v_54 = v_3(v_47, v_48, v_49, v_50, v_45);
      v_52 = max(ceil(sqrt((3.0f * sqrt(v_54)))), 1.0f);
    }
  } else {
    bool v_55 = false;
    if (all((v_47 == v_48))) {
      v_55 = true;
    } else {
      v_55 = all((v_48 == v_49));
    }
    if (v_55) {
      v_52 = 1.0f;
    } else {
      float const v_56 = v_11((v_45 * v_47), (v_45 * v_48), (v_45 * v_49), v_51);
      v_52 = max(ceil(sqrt(v_56)), 1.0f);
    }
  }
  float v_57 = v_37.tint_member_4.x;
  float v_58 = v_37.tint_member_4.y;
  bool const v_59 = (v_37.tint_member_4.x == 0.0f);
  float v_60 = 0.0f;
  if (v_59) {
    v_60 = 0.47746482491493225098f;
    v_57 = 0.5f;
  } else {
    v_60 = (0.5f / acos(max((1.0f - (0.25f / ((*v_39.tint_member_9).tint_member_10[2].z * v_37.tint_member_4.x))), -1.0f)));
  }
  if (v_59) {
    v_47 = (v_45 * v_47);
    v_48 = (v_45 * v_48);
    v_49 = (v_45 * v_49);
    v_50 = (v_45 * v_50);
    v_46 = (v_45 * v_46);
  }
  float2 v_61 = 0.0f;
  if (all((v_47 == v_48))) {
    float2 const v_62 = v_49;
    float2 const v_63 = v_50;
    v_61 = select(v_62, v_63, bool2(all((v_48 == v_49))));
  } else {
    v_61 = v_48;
  }
  float2 v_64 = v_27(v_61, v_47);
  float2 v_65 = 0.0f;
  if (all((v_50 == v_49))) {
    float2 const v_66 = v_48;
    float2 const v_67 = v_47;
    v_65 = select(v_66, v_67, bool2(all((v_49 == v_48))));
  } else {
    v_65 = v_49;
  }
  float2 v_68 = v_27(v_50, v_65);
  float2 v_69 = v_27(v_47, v_46);
  if (all((v_64 == float2(0.0f)))) {
    v_58 = 0.0f;
    if ((v_51 < 0.0f)) {
      v_64 = float2(1.0f, 0.0f);
      v_68 = float2(-1.0f, 0.0f);
    } else {
      v_51 = -1.0f;
      v_64 = v_69;
      v_68 = v_69;
      if (all((v_69 == float2(0.0f)))) {
        v_50 = (v_47 + (v_57 * float2(1.0f, 0.0f)));
        v_49 = v_50;
        v_48 = (v_47 - (v_57 * float2(1.0f, 0.0f)));
        v_47 = v_48;
        v_68 = float2(1.0f, 0.0f);
        v_64 = v_68;
        v_69 = v_64;
      } else {
        v_50 = (v_47 + (v_57 * v_69));
        v_49 = v_50;
      }
    }
  }
  float v_70 = 0.0f;
  if ((v_58 >= 0.0f)) {
    v_70 = (sign(v_58) + 3.0f);
  } else {
    float const v_71 = acos(clamp(dot(v_69, v_64), -1.0f, 1.0f));
    float const v_72 = max(ceil((v_71 * v_60)), 1.0f);
    v_70 = (v_72 + 2.0f);
    v_70 = min(v_70, 16381.0f);
  }
  float2 const v_73 = (v_49 - v_47);
  float2 const v_74 = (v_50 - v_48);
  float2 const v_75 = float2(v_73.x, v_73.y);
  float v_76 = determinant(float2x2(v_75, float2(v_74.x, v_74.y)));
  float v_77 = (abs(v_40) - v_70);
  if ((v_77 < 0.0f)) {
    v_68 = v_64;
    if (any((v_46 != v_47))) {
      v_64 = v_69;
    }
    float const v_78 = v_68.x;
    float const v_79 = v_68.y;
    float2 const v_80 = float2(v_64.x, v_64.y);
    v_76 = determinant(float2x2(v_80, float2(v_78, v_79)));
  }
  float const v_81 = clamp(dot(v_64, v_68), -1.0f, 1.0f);
  float v_82 = acos(v_81);
  if ((v_76 < 0.0f)) {
    v_82 = -(v_82);
  }
  float v_83 = 0.0f;
  float v_84 = sign(v_40);
  if ((v_77 < 0.0f)) {
    v_83 = (v_70 - 2.0f);
    v_52 = 1.0f;
    v_48 = v_47;
    v_49 = v_48;
    v_50 = v_49;
    v_77 = (v_77 + (v_83 + 1.0f));
    if ((v_77 < 0.0f)) {
      v_77 = 0.0f;
    } else {
      bool const v_85 = ((abs(v_76) * rsqrt((dot(v_64, v_64) * dot(v_68, v_68)))) < 0.00999999977648258209f);
      bool v_86 = false;
      if (!(v_85)) {
        v_86 = true;
      } else {
        v_86 = (dot(v_64, v_68) < 0.0f);
      }
      if (v_86) {
        float v_87 = 0.0f;
        if ((v_76 < 0.0f)) {
          v_87 = min(v_84, 0.0f);
        } else {
          v_87 = max(v_84, 0.0f);
        }
        v_84 = v_87;
      }
    }
  } else {
    float const v_88 = ((16383.0f - v_70) - 1.0f);
    v_83 = max(ceil((abs(v_82) * v_60)), 1.0f);
    v_83 = min(v_83, v_88);
    v_52 = min(v_52, ((v_88 - v_83) + 1.0f));
  }
  float const v_89 = (v_82 / v_83);
  float const v_90 = ((v_52 + v_83) - 1.0f);
  bool const v_91 = (v_77 >= v_90);
  if ((v_77 > v_90)) {
    v_84 = 0.0f;
  }
  bool v_92 = false;
  if ((abs(v_40) == 2.0f)) {
    v_92 = (v_58 > 0.0f);
  } else {
    v_92 = false;
  }
  if (v_92) {
    float const v_93 = fma(v_81, 0.5f, 0.5f);
    float v_94 = 0.0f;
    if ((((v_93 * v_58) * v_58) >= 1.0f)) {
      v_94 = rsqrt(v_93);
    } else {
      v_94 = sqrt(v_93);
    }
    v_84 = (v_84 * v_94);
  }
  float2 v_95 = 0.0f;
  float2 v_96 = 0.0f;
  bool v_97 = false;
  if ((v_77 != 0.0f)) {
    v_97 = !(v_91);
  } else {
    v_97 = false;
  }
  if (v_97) {
    float2 v_98 = 0.0f;
    float2 v_99 = 0.0f;
    float2 v_100 = (v_48 - v_47);
    float2 const v_101 = (v_50 - v_47);
    if ((v_51 >= 0.0f)) {
      v_100 = (v_100 * v_51);
      v_99 = ((0.5f * v_101) - v_100);
      v_98 = ((v_51 - 1.0f) * v_101);
      v_48 = (v_48 * v_51);
    } else {
      float2 const v_102 = (v_49 - v_48);
      v_99 = (v_102 - v_100);
      v_98 = fma(float2(-3.0f), v_102, v_101);
    }
    float2 const v_103 = (v_99 * (v_52 * 2.0f));
    float2 const v_104 = (v_100 * (v_52 * v_52));
    float v_105 = 0.0f;
    float const v_106 = min((v_52 - 1.0f), v_77);
    float const v_107 = -(abs(v_89));
    float const v_108 = ((1.0f + v_77) * abs(v_89));
    float v_109 = 32.0f;
    {
      while(true) {
        if ((v_109 >= 1.0f)) {
        } else {
          break;
        }
        float const v_110 = (v_105 + v_109);
        if ((v_110 <= v_106)) {
          float2 const v_111 = float2(v_110);
          float2 v_112 = fma(v_111, v_98, v_103);
          float2 const v_113 = float2(v_110);
          v_112 = fma(v_113, v_112, v_104);
          float const v_114 = dot(normalize(v_112), v_64);
          float v_115 = fma(v_110, v_107, v_108);
          v_115 = min(v_115, 3.14159274101257324219f);
          if ((v_114 >= cos(v_115))) {
            v_105 = v_110;
          }
        }
        {
          v_109 = (v_109 * 0.5f);
        }
      }
    }
    float const v_116 = (v_105 / v_52);
    float const v_117 = (v_77 - v_105);
    float v_118 = acos(clamp(v_64.x, -1.0f, 1.0f));
    v_118 = select(-(v_118), v_118, (v_64.y >= 0.0f));
    float const v_119 = fma(v_117, v_89, v_118);
    v_95 = float2(cos(v_119), sin(v_119));
    float2 const v_120 = float2(-(v_95.y), v_95.x);
    float const v_121 = dot(v_120, v_98);
    float const v_122 = dot(v_120, v_99);
    float const v_123 = dot(v_120, v_100);
    float const v_124 = max(((v_122 * v_122) - (v_121 * v_123)), 0.0f);
    float v_125 = sqrt(v_124);
    if ((v_122 > 0.0f)) {
      v_125 = -(v_125);
    }
    v_125 = (v_125 - v_122);
    float const v_126 = ((-0.5f * v_125) * v_121);
    float2 v_127 = 0.0f;
    if ((abs(fma(v_125, v_125, v_126)) < abs(fma(v_121, v_123, v_126)))) {
      v_127 = float2(v_125, v_121);
    } else {
      v_127 = float2(v_123, v_125);
    }
    float2 const v_128 = v_127;
    float v_129 = 0.0f;
    bool v_130 = false;
    if ((v_117 != 0.0f)) {
      v_130 = (v_128.y != 0.0f);
    } else {
      v_130 = false;
    }
    if (v_130) {
      v_129 = saturate((v_128.x / v_128.y));
    } else {
      v_129 = 0.0f;
    }
    float const v_131 = v_129;
    float const v_132 = max(v_116, v_131);
    float2 const v_133 = v_32(v_47, v_48, v_132);
    float2 const v_134 = v_32(v_48, v_49, v_132);
    float2 const v_135 = v_32(v_49, v_50, v_132);
    float2 const v_136 = v_32(v_133, v_134, v_132);
    float2 const v_137 = v_32(v_134, v_135, v_132);
    float2 const v_138 = v_32(v_136, v_137, v_132);
    float const v_139 = fma((v_51 - 1.0f), v_132, 1.0f);
    float const v_140 = ((v_51 + 1.0f) - v_139);
    float const v_141 = fma((v_140 - v_139), v_132, v_139);
    if ((v_132 != v_131)) {
      float2 v_142 = 0.0f;
      if ((v_51 >= 0.0f)) {
        v_142 = v_27((v_134 * v_139), (v_133 * v_140));
      } else {
        v_142 = v_27(v_137, v_136);
      }
      v_95 = v_142;
    }
    float2 v_143 = 0.0f;
    if ((v_51 >= 0.0f)) {
      v_143 = (v_136 / v_141);
    } else {
      v_143 = v_138;
    }
    v_96 = v_143;
  } else {
    float2 const v_144 = v_68;
    float2 const v_145 = v_64;
    v_95 = select(v_144, v_145, bool2((v_77 == 0.0f)));
    float2 const v_146 = v_50;
    float2 const v_147 = v_47;
    v_96 = select(v_146, v_147, bool2((v_77 == 0.0f)));
  }
  float2 const v_148 = float2(v_95.y, -(v_95.x));
  v_96 = (v_96 + (v_148 * (v_57 * v_84)));
  if (v_59) {
    float2 const v_149 = (v_96 + (*v_39.tint_member_9).tint_member_10[2].xy);
    float2x2 const v_150 = v(v_45);
    v_41 = float4(v_149, (v_150 * v_96));
  } else {
    v_41 = float4(((v_45 * v_96) + (*v_39.tint_member_9).tint_member_10[2].xy), v_96);
  }
  float4 const v_151 = v_41;
  float4 const v_152 = float4(v_151.xy, v_37.tint_member_5, 1.0f);
  (*v_38).tint_member_8 = float4((((*v_39.tint_member_9).tint_member_10[0].zw * v_152.xy) - (sign((*v_39.tint_member_9).tint_member_10[0].zw) * v_152.ww)), v_152.zw);
}

tint_struct_1 v_153(tint_struct v_154, tint_struct_2 v_155) {
  tint_struct_1 v_156 = {};
  (v_36(v_154, (&v_156), v_155));
  return v_156;
}

vertex tint_struct_4 tint_entry_point(uint v_158 [[vertex_id]], tint_struct_5 v_159 [[stage_in]], const constant tint_struct_3* v_160 [[buffer(0)]]) {
  tint_struct_2 const v_161 = tint_struct_2{.tint_member_9=v_160};
  tint_struct_4 v_162 = {};
  v_162.tint_member_11 = v_153(tint_struct{.tint_member=v_158, .tint_member_1=v_159.tint_member_12, .tint_member_2=v_159.tint_member_13, .tint_member_3=v_159.tint_member_14, .tint_member_4=v_159.tint_member_15, .tint_member_5=v_159.tint_member_16, .tint_member_6=v_159.tint_member_17, .tint_member_7=v_159.tint_member_18}, v_161).tint_member_8;
  return v_162;
}
