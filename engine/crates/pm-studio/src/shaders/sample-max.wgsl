// Read back one integer for global stopping, rather than the full RGBA film.
@group(0) @binding(0) var<uniform> dims:vec4u;
@group(0) @binding(1) var<storage,read> samples:array<vec4f>;
@group(0) @binding(2) var<storage,read_write> result:atomic<u32>;
var<workgroup> maximum:atomic<u32>;
@compute @workgroup_size(16,16)
fn main(@builtin(global_invocation_id) p:vec3u,@builtin(local_invocation_index) local:u32){
  if(local==0u){atomicStore(&maximum,0u);}workgroupBarrier();
  var count=0u;if(all(p.xy<dims.xy)){count=u32(samples[p.y*dims.x+p.x].w);}
  atomicMax(&maximum,count);workgroupBarrier();
  if(local==0u){atomicMax(&result,atomicLoad(&maximum));}
}
