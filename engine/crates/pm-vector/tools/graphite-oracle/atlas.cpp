// Synthetic RectanizerSkyline records; no fonts or captured glyph dimensions.
// Compile with src/gpu/RectanizerSkyline.cpp and the pinned Skia CPU library.
#include "src/gpu/RectanizerSkyline.h"
#include "src/core/SkIPoint16.h"
#include <cstdio>
int main() {
    skgpu::RectanizerSkyline skyline(512, 512);
    constexpr int sizes[][2] = {{47,61},{35,96},{203,97},{131,171},{256,256},
                               {9,190},{300,128},{1,512},{513,1},{71,37}};
    for (int i=0; i<30; ++i) {
        const auto& size = sizes[i%10];
        SkIPoint16 pos;
        if (skyline.addRect(size[0],size[1],&pos))
            std::printf("%d %d\n", pos.fX, pos.fY);
        else std::puts("none");
    }
}
