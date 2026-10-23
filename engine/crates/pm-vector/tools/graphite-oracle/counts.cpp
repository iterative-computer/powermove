// Independent CPU segment-count oracle, Skia chrome/m152 747cf654.
#include "include/core/SkMatrix.h"
#include "src/gpu/tessellate/LinearTolerances.h"
#include "src/gpu/tessellate/FixedCountBufferUtils.h"
#include "src/gpu/tessellate/WangsFormula.h"
#include <algorithm>
#include <iostream>
int main() {
    using namespace skgpu::tess;
    namespace wangs_formula=skgpu::wangs_formula;
    SkPoint q[]={{0,0},{0,100},{100,100}};
    SkPoint c[]={{0,0},{0,50},{50,100},{100,100}};
    for(float scale : {1.f,5.f}) {
        auto vx=wangs_formula::VectorXform(SkMatrix::Scale(scale,scale));
        float conic=wangs_formula::conic_p2(kPrecision,q,0.70710677f,vx);
        float n4s[]={1.f,wangs_formula::quadratic_p4(kPrecision,q,vx),wangs_formula::cubic_p4(kPrecision,c,vx),conic*conic};
        for(float join : {-1.f,0.f,2.f}) {
            for(float n4 : n4s) {
                LinearTolerances t;
                t.setParametricSegments(std::max(1.f,std::min(n4,float(kMaxParametricSegments_p4))));
                t.setStroke(StrokeParams(4.5f,join),scale);
                std::cout<<FixedCountStrokes::VertexCount(t)<<",";
            }
            std::cout<<"\n";
        }
        for(float n4:n4s) { LinearTolerances t; t.setParametricSegments(std::max(1.f,std::min(n4,float(kMaxParametricSegments_p4)))); std::cout<<FixedCountWedges::VertexCount(t)<<","; }
        std::cout<<"\n";
    }
}
