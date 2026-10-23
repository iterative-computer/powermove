// Float transform-operation oracle for Chromium 152's SkCanvas/SkM44.
// Build/link as counts.cpp. No project data or fonts required.
#include "include/core/SkM44.h"
#include "include/core/SkMatrix.h"
#include <bit>
#include <cstdio>
static void print(const SkM44&m) {
    float a[16];m.getColMajor(a);
    for(int i:{0,1,4,5,12,13})std::printf("0x%08x,",std::bit_cast<unsigned>(a[i]));
    std::puts("");
}
int main(){
    SkM44 caption;
    caption.preScale(1.25f,1.25f);caption.preTranslate(27.3f,71.9f);
    caption.preTranslate(460.13f,52.08f);caption.preScale(1.16f,1.16f);
    caption.preTranslate(-460.13f,-52.08f);print(caption);
    SkMatrix start;start.setAll(1.75f,0,-345.123f,0,1.75f,-87.314f,0,0,1);
    SkM44 animated(start);animated.preTranslate(732.123f,422.91f);
    SkMatrix glyph;glyph.setAll(.75f,-.18f,3.37f,.125f,.91f,8.81f,0,0,1);
    animated.preConcat(glyph);print(animated);
}
