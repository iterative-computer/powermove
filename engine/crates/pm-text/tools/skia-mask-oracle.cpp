// Synthetic A8 reference generator for tests/analytic_masks.rs.
// Build against Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5,
// with SK_SUPPORT_LEGACY_CONIC_CHOP and -ffp-contract=off (Chromium config).
// Input: width height, then M/L/Q/K/C/Z and f32 coordinates, one verb per line.
// Output: tightly packed A8 bytes; the fixture stores FNV-1a hashes only.
#include "include/core/SkBitmap.h"
#include "include/core/SkCanvas.h"
#include "include/core/SkPathBuilder.h"
#include "include/core/SkPaint.h"
#include "include/core/SkImageInfo.h"
#include "include/core/SkPathUtils.h"
#include <fstream>
#include <iostream>
int main(int argc, char**argv) {
 if(argc != 3) return 2;
 std::ifstream in(argv[1]); int w,h; in>>w>>h; SkPathBuilder p; char verb; float x,y,a,b,c,d,k;
 while(in>>verb) { switch(verb) {
 case 'M':in>>x>>y;p.moveTo(x,y);break;
 case 'L':in>>x>>y;p.lineTo(x,y);break;
 case 'Q':in>>a>>b>>x>>y;p.quadTo(a,b,x,y);break;
 case 'K':in>>a>>b>>x>>y>>k;p.conicTo(a,b,x,y,k);break;
 case 'C':in>>a>>b>>c>>d>>x>>y;p.cubicTo(a,b,c,d,x,y);break;
 case 'Z':p.close();break;
 default:return 2;
 }}
 SkBitmap bitmap; bitmap.allocPixels(SkImageInfo::MakeA8(w,h));bitmap.eraseColor(SK_ColorTRANSPARENT);
 SkCanvas canvas(bitmap); SkPaint paint;paint.setAntiAlias(true);paint.setColor(SK_ColorWHITE);
 auto path=p.detach();canvas.drawPath(path,paint);
 std::ofstream out(argv[2],std::ios::binary);
 for(int y=0;y<h;y++)out.write((char*)bitmap.getAddr8(0,y),w);
}
