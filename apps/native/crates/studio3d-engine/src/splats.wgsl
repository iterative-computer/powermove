struct Vertex { @location(0) center:vec2<f32>, @location(1) major:vec2<f32>, @location(2) minor:vec2<f32>, @location(3) color:vec4<f32> }
struct Varying { @builtin(position) position:vec4<f32>, @location(0) color:vec4<f32>, @location(1) point:vec2<f32> }
@vertex fn vs(v:Vertex,@builtin(vertex_index) index:u32)->Varying {
    let p=vec2<f32>(f32(index&1u),f32(index>>1u))*4.-2.;
    let at=v.center+p.x*v.major+p.y*v.minor;
    var out:Varying;
    // Texture row zero is the FBO's bottom row, as in the compositor.
    out.position=vec4<f32>(at.x,-at.y,0.,1.);out.color=v.color;out.point=p;return out;
}
@fragment fn fs(v:Varying)->@location(0) vec4<f32> {
    let r=-dot(v.point,v.point);if r< -4. {discard;}
    let alpha=exp(r)*v.color.a;if alpha<1./255. {discard;}
    return vec4<f32>(v.color.rgb*alpha,alpha);
}
