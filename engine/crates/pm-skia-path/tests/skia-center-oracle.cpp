// Reference generator for convex_stroke_and_fill_matches_skia_152_path_records.
// Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5, -ffp-contract=off.
// Stroke-and-fill of convex line-only closed contours: SkStroke::strokePath
// sets ignoreCenter and keeps only the larger offset.
#include "include/core/SkPath.h"
#include "include/core/SkPathBuilder.h"
#include "include/core/SkPathUtils.h"
#include "include/core/SkPaint.h"
#include <bit>
#include <cstdio>
int main(){
 const float poly[5][5][2]={
  {{2.5f,3.25f},{20.75f,3.25f},{14.125f,30.5f},{-4.125f,30.5f}},
  {{2.5f,3.25f},{-4.125f,30.5f},{14.125f,30.5f},{20.75f,3.25f}},
  {{1.f,1.f},{17.5f,4.25f},{6.375f,19.f}},
  {{1.f,1.f},{6.375f,19.f},{17.5f,4.25f}},
  {{10.f,0.5f},{0.75f,7.25f},{4.25f,18.5f},{15.75f,18.5f},{19.25f,7.25f}},
 };
 const int n[5]={4,4,3,3,5};
 const float width[5]={3.75f,3.75f,2.f,5.5f,3.f};
 const float miter[5]={4.f,4.f,4.f,4.f,1.5f};
 const SkPaint::Join join[5]={SkPaint::kMiter_Join,SkPaint::kMiter_Join,SkPaint::kBevel_Join,
                              SkPaint::kRound_Join,SkPaint::kMiter_Join};
 for(int k=0;k<5;k++){
 SkPathBuilder b;b.moveTo(poly[k][0][0],poly[k][0][1]);
 for(int i=1;i<n[k];i++)b.lineTo(poly[k][i][0],poly[k][i][1]);
 b.close();auto p=b.detach();
 SkPaint paint;paint.setStyle(SkPaint::kStrokeAndFill_Style);
 paint.setStrokeWidth(width[k]);paint.setStrokeJoin(join[k]);paint.setStrokeMiter(miter[k]);
 SkPathBuilder out;skpathutils::FillPathWithPaint(p,paint,&out);auto q=out.detach();
 unsigned long long hash=14695981039346656037ull;unsigned count=0;
 auto add=[&](unsigned v){hash^=v;hash*=1099511628211ull;count++;};
 SkPath::Iter it(q,false);while(auto rec=it.next()){
 add(unsigned(rec->fVerb));
 for(auto pt:rec->fPoints){add(std::bit_cast<unsigned>(pt.fX));add(std::bit_cast<unsigned>(pt.fY));}
 if(rec->fVerb==SkPathVerb::kConic)add(std::bit_cast<unsigned>(rec->conicWeight()));
 }
 printf("%d %u %llu %d\n",k,count,hash,p.isConvex());
 }
}
