// A deterministic pinhole pick uses the tracer's material/alpha/normal-map
// evaluation. No frame or geometry readback is needed.
@group(2) @binding(0) var<storage,read_write> query_out:array<vec4f>;
@group(2) @binding(1) var<uniform> query_pixel:vec4f;
@compute @workgroup_size(1)
fn pick_surface() {
  query_out[0]=vec4f(-1.);query_out[1]=vec4f(0.);query_out[2]=vec4f(0.);
  seed=1u;let ray=camera_ray(query_pixel.xy*vec2f(P.dims.xy));var ro=ray[0];let rd=ray[1];
  for(var step=0u;step<24u;step++){
    let h=primary_intersect(ro,rd,camera_limit(ro,rd));
    if(h.tri<0||floor_t(ro,rd)<h.t){return;}
    let sf=surface(ro,rd,h);let front=dot(sf.ng,rd)<=0.;
    if(sf.m.extra.z<.5||(!front&&(u32(sf.m.policy.y)&4u)==0u)){ro=sf.p+rd*ray_epsilon();continue;}
    let n=select(-sf.n,sf.n,front);
    query_out[0]=vec4f(f32(sf.object),f32(h.inst),f32(h.tri),distance(sf.p,P.cam_pos.xyz));
    query_out[1]=vec4f(sf.p,1.);query_out[2]=vec4f(n,0.);return;
  }
}
