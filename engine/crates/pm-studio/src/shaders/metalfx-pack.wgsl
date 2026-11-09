struct Pack { dims:vec4u, mask:vec4u, backdrop:vec4f }
@group(0) @binding(0) var<uniform> Q:Pack;
@group(0) @binding(1) var color:texture_2d<f32>;
@group(0) @binding(2) var masks:texture_2d<f32>;
@group(0) @binding(3) var<storage,read_write> packed:array<u32>;
@group(0) @binding(4) var frame_image:texture_storage_2d<rgba8unorm,write>;
fn neutral(c0:vec3f)->vec3f {
 var c=c0;let x=min(c.x,min(c.y,c.z));c-=select(.04,x-6.25*x*x,x<.08);let peak=max(c.x,max(c.y,c.z));
 if(peak<.76){return c;}let np=1.-.24*.24/(peak+.24-.76);c*=np/peak;return mix(c,vec3f(np),1.-1./(.15*(peak-np)+1.));
}
fn encode(c:vec3f)->vec3f {return select(1.055*pow(max(c,vec3f(0.)),vec3f(1./2.4))-.055,c*12.92,c<=vec3f(.0031308));}
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) g:vec3u) {
 if(g.x>=Q.dims.x||g.y>=Q.dims.y){return;}
 let pixel=vec2i(g.xy);let input=vec2i(min(g.xy*Q.dims.zw/Q.dims.xy,Q.dims.zw-1u));let m=textureLoad(masks,input,0);let linear=textureLoad(color,pixel,0).xyz;
 var c=vec4f(encode(clamp(neutral(linear),vec3f(0.),vec3f(1.))),1.);
 if(m.x<.5){c=vec4f(encode(clamp(linear,vec3f(0.),vec3f(1.))),Q.backdrop.w);}
 if(Q.mask.x>0u&&u32(m.z+.5)!=Q.mask.x){if(m.z>.5||Q.mask.y==0u){c=vec4f(0.);}}
 if(Q.backdrop.w<.5&&m.x<.5){c=vec4f(0.);}
 textureStore(frame_image,pixel,clamp(c,vec4f(0.),vec4f(1.)));
 let q=vec4u(clamp(c,vec4f(0.),vec4f(1.))*255.+.5);packed[g.y*Q.dims.x+g.x]=q.x|(q.y<<8u)|(q.z<<16u)|(q.w<<24u);
}
