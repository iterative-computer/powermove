// Synthetic atlas-byte oracle for Skia chrome/m152. No font data required.
// Build/link as counts.cpp; invoke with no arguments.
#include "src/core/SkDistanceFieldGen.h"
#include <cstdint>
#include <cstdio>
#include <vector>
int main() {
    for (int seed = 0; seed < 8; ++seed) {
        int w = 11 + seed * 3, h = 9 + seed * 2;
        std::vector<unsigned char> a8(w*h), df((w+8)*(h+8));
        for (int y=0;y<h;++y) for(int x=0;x<w;++x) {
            a8[y*w+x] = (x*17+y*43+x*y*7+seed*31)&255;
        }
        if (!SkGenerateDistanceFieldFromA8Image(df.data(),a8.data(),w,h,w)) return 1;
        uint64_t hash=14695981039346656037ull;
        for (auto b:df) { hash^=b;hash*=1099511628211ull; }
        std::printf("%d %d %d %016llx\n",seed,w,h,(unsigned long long)hash);
    }
}
