// Reference generator for rect_contours_match_skia_152 (tests/skia_rect_contours.rs).
// Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5, -ffp-contract=off.
// Each case is the verb string in skia-rect-contour-cases.inc; prints
// SkPath::isRect as "0" or "1 left top right bottom closed cw".
#include "include/core/SkPath.h"
#include "include/core/SkPathBuilder.h"
#include <cstdio>
#include <sstream>
#include <string>
static const char* kCases[] = {
#include "skia-rect-contour-cases.inc"
};
int main(){
 for(const char* c:kCases){
  std::istringstream in(c);SkPathBuilder b;std::string v;float x,y;
  while(in>>v){
   if(v=="M"){in>>x>>y;b.moveTo(x,y);}
   else if(v=="L"){in>>x>>y;b.lineTo(x,y);}
   else if(v=="Z"){b.close();}
   else return 2;
  }
  SkRect r;bool closed=false;SkPathDirection dir;
  if(b.detach().isRect(&r,&closed,&dir))
   printf("1 %g %g %g %g %d %d\n",r.fLeft,r.fTop,r.fRight,r.fBottom,closed,dir==SkPathDirection::kCW);
  else printf("0\n");
 }
}
