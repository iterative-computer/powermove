// N32 solid-span reference generator for tests/analytic_masks.rs.
// Pinned Skia/configuration are in fixtures/skia-analytic-masks.json.
// Input: width height left top, then M/L/Q/K/C/Z paths, each followed
// by D red green blue alpha. Output: premultiplied RGBA8.
#include "include/core/SkBitmap.h"
#include "include/core/SkCanvas.h"
#include "include/core/SkPathBuilder.h"
#include "include/core/SkPaint.h"
#include "include/core/SkImageInfo.h"
#include "src/core/SkColorPriv.h"
#include <fstream>
int main(int argc,char**argv) {
 std::ifstream in(argv[1]); int w,h,l,t;in>>w>>h>>l>>t;
 SkBitmap bm;bm.allocPixels(SkImageInfo::MakeN32Premul(w,h));bm.eraseColor(SK_ColorTRANSPARENT);
 SkCanvas canvas(bm);canvas.translate(-l,-t);SkPathBuilder p;char v;float x,y,a,b,c,d,k;
 while(in>>v){switch(v){
 case 'M':in>>x>>y;p.moveTo(x,y);break;
 case 'L':in>>x>>y;p.lineTo(x,y);break;
 case 'Q':in>>a>>b>>x>>y;p.quadTo(a,b,x,y);break;
 case 'K':in>>a>>b>>x>>y>>k;p.conicTo(a,b,x,y,k);break;
 case 'C':in>>a>>b>>c>>d>>x>>y;p.cubicTo(a,b,c,d,x,y);break;
 case 'Z':p.close();break;
 case 'D':in>>a>>b>>c>>d;{SkPaint paint;paint.setAntiAlias(true);paint.setColor4f({a,b,c,d},nullptr);canvas.drawPath(p.detach(),paint);}break;
 default:return 2;
 }}
 std::ofstream out(argv[2],std::ios::binary);
 for(int y=0;y<h;y++)for(int x=0;x<w;x++){auto c=*bm.getAddr32(x,y);unsigned char px[]={static_cast<unsigned char>(SkGetPackedR32(c)),static_cast<unsigned char>(SkGetPackedG32(c)),static_cast<unsigned char>(SkGetPackedB32(c)),static_cast<unsigned char>(SkGetPackedA32(c))};out.write((char*)px,4);}
}
