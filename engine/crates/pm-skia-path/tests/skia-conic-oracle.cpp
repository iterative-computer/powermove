// Reference generator for tests/skia_conics.rs: build against Skia chrome/m152
// 747cf654ed6f864f4402e7d6eb54a6ede60141e5 with -ffp-contract=off.
#include "include/core/SkPath.h"
#include "include/core/SkPathBuilder.h"
#include "include/core/SkPathUtils.h"
#include "include/core/SkPaint.h"
#include <bit>
#include <cstdio>
int main(){
 for(int k=0;k<5;k++){
 SkPathBuilder b;
 if(k==0){b.moveTo(0,20);b.conicTo(0,0,20,0,0.7071067811865475f);}
 if(k==1){b.moveTo(3.25f,19.5f);b.conicTo(27.125f,-8.75f,41.75f,31.25f,2.5f);}
 if(k==2){b.moveTo(1,1);b.conicTo(9,1,4,1,0.5f);}
 if(k==3){b.moveTo(1,1);b.conicTo(1,1,10,8,0.5f);}
 if(k==4){b.moveTo(-2.75f,9.125f);b.conicTo(11.5f,-3.25f,26.625f,17.875f,0.125f);b.lineTo(31,20);b.close();}
 auto p=b.detach();SkPaint paint;paint.setStyle(SkPaint::kStroke_Style);paint.setStrokeWidth(3.75f);paint.setStrokeJoin(SkPaint::kRound_Join);paint.setStrokeCap(SkPaint::kRound_Cap);SkPathBuilder out;skpathutils::FillPathWithPaint(p,paint,&out);auto q=out.detach();
 unsigned long long hash=14695981039346656037ull;unsigned count=0;
 auto add=[&](unsigned v){hash^=v;hash*=1099511628211ull;count++;};
 SkPath::Iter it(q,false);while(auto rec=it.next()){
 add(unsigned(rec->fVerb));
 for(auto pt:rec->fPoints){add(std::bit_cast<unsigned>(pt.fX));add(std::bit_cast<unsigned>(pt.fY));}
 if(rec->fVerb==SkPathVerb::kConic)add(std::bit_cast<unsigned>(rec->conicWeight()));
 }
 printf("%d %u %llu\n",k,count,hash);
 }
}
