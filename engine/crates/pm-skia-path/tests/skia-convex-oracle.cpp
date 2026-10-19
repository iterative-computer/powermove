// Reference generator for convexity_matches_skia_152 (tests/skia_convexity.rs).
// Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5, -ffp-contract=off.
// Each case is the same verb string as in the Rust test; prints isConvex().
#include "include/core/SkPath.h"
#include "include/core/SkPathBuilder.h"
#include <cstdio>
#include <sstream>
#include <string>
static const char* kCases[] = {
#include "skia-convex-cases.inc"
};
int main(){
 for(const char* c:kCases){
  std::istringstream in(c);SkPathBuilder b;std::string v;float x,y,a,d,e,f;
  while(in>>v){
   if(v=="M"){in>>x>>y;b.moveTo(x,y);}
   else if(v=="L"){in>>x>>y;b.lineTo(x,y);}
   else if(v=="Q"){in>>a>>d>>x>>y;b.quadTo(a,d,x,y);}
   else if(v=="C"){in>>a>>d>>e>>f>>x>>y;b.cubicTo(a,d,e,f,x,y);}
   else if(v=="Z"){b.close();}
   else return 2;
  }
  printf("%d\n",b.detach().isConvex()?1:0);
 }
}
