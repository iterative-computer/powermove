// Reference generator for rectangle_frames_match_skia_152_path_records.
// Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5, -ffp-contract=off.
#include "include/core/SkPath.h"
#include "include/core/SkPathBuilder.h"
#include "include/core/SkPathUtils.h"
#include "include/core/SkPaint.h"
#include <bit>
#include <cstdio>
int main(){
 for(int k=0;k<5;k++){
 SkPathBuilder b;
 b.moveTo(2.125f,3.75f);
 if(k==4){b.lineTo(2.125f,45.125f);b.lineTo(14.375f,45.125f);b.lineTo(14.375f,3.75f);}
 else {b.lineTo(14.375f,3.75f);b.lineTo(14.375f,45.125f);b.lineTo(2.125f,45.125f);}
 b.close();auto p=b.detach();
 SkPaint paint;paint.setStyle(k==3?SkPaint::kStrokeAndFill_Style:SkPaint::kStroke_Style);
 paint.setStrokeWidth(k==0?14.0f:k==1?2.0f:3.75f);
 paint.setStrokeJoin(k==2?SkPaint::kMiter_Join:k==3?SkPaint::kBevel_Join:SkPaint::kRound_Join);
 paint.setStrokeMiter(k==2?0.5f:4.0f);
 SkPathBuilder out;skpathutils::FillPathWithPaint(p,paint,&out);auto q=out.detach();
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
