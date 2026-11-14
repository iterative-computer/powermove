struct Camera { eye:vec4f, forward:vec4f, right:vec4f, up:vec4f, clip:vec4f, ambient:vec4f, backdrop:vec4f }
struct Instance { world:mat4x4f, normal:mat4x4f }
struct Material { base:vec4f, spec:vec4f, emit:vec4f, extra:vec4f, maps:vec4f, layers:vec4f, policy:vec4f, maps2:vec4f, imported:array<vec4f,6>, physical_maps:array<TextureMap,18> }
struct TextureMap { info:vec4f, u:vec4f, v:vec4f }
struct Light { pos:vec4f, u:vec4f, v:vec4f, radiance:vec4f }
@group(0) @binding(0) var<uniform> camera:Camera;
@group(0) @binding(1) var<storage,read> materials:array<Material>;
@group(0) @binding(2) var<storage,read> instances:array<Instance>;
@group(0) @binding(3) var<storage,read> lights:array<Light>;
@group(0) @binding(4) var maps:texture_2d_array<f32>;
@group(0) @binding(5) var smp:sampler;
struct Vary { @builtin(position) clip:vec4f, @location(0) world:vec3f, @location(1) normal:vec3f, @location(2) uv:vec2f, @location(3) uv1:vec2f, @location(4) color:vec4f, @location(5) @interpolate(flat) material:u32, @location(6) @interpolate(flat) mirrored:u32 }
@vertex fn vs(@location(0) position:vec3f,@location(1) normal:vec3f,@location(2) uv:vec2f,@location(3) uv1:vec2f,@location(4) color:vec4f,@location(5) material:u32,@builtin(instance_index) instance:u32)->Vary {
 let world=(instances[instance].world*vec4f(position,1.)).xyz;
 let delta=world-camera.eye.xyz;let z=dot(delta,camera.forward.xyz);
 let ortho=camera.clip.z>0.;let w=select(z,1.,ortho);
 let near=camera.clip.x;let far=camera.clip.y;
 let depth=select(far*(z-near)/(far-near),(z-near)/(far-near),ortho);
 var o:Vary;o.clip=vec4f(dot(delta,camera.right.xyz)/camera.right.w,dot(delta,camera.up.xyz)/camera.up.w*camera.clip.w,depth,w);
 o.world=world;o.normal=(instances[instance].normal*vec4f(normal,0.)).xyz;o.uv=uv;o.uv1=uv1;o.color=color;o.material=material;o.mirrored=select(0u,1u,instances[instance].normal[3].w<0.);return o;
}
fn decode(c:vec3f)->vec3f { return select(c/12.92,pow((c+0.055)/1.055,vec3f(2.4)),c>vec3f(0.04045)); }
fn encode(c:vec3f)->vec3f {return select(c*12.92,1.055*pow(max(c,vec3f(0.)),vec3f(1./2.4))-0.055,c>vec3f(0.0031308));}
fn tex(layer:f32,uv:vec2f)->vec4f {if(layer<0.){return vec4f(1.);}return textureSampleLevel(maps,smp,fract(uv),i32(layer),0.);}
fn wrap(v:f32,mode:f32,dim:f32)->f32 {if(mode<0.5){return fract(v);}if(mode<1.5){return clamp(v,0.5/dim,1.-0.5/dim);}return clamp(1.-abs(2.*fract(v*0.5)-1.),0.5/dim,1.-0.5/dim);}
fn physical(m:Material,slot:u32,i:Vary)->vec4f {let t=m.physical_maps[slot];if(t.info.x<0.){return vec4f(1.);}let uv=vec3f(select(i.uv,i.uv1,t.info.y>0.5),1.);let at=vec2f(dot(t.u.xyz,uv),dot(t.v.xyz,uv));let d=vec2f(textureDimensions(maps));return textureSampleLevel(maps,smp,vec2f(wrap(at.x,t.info.z,d.x),wrap(at.y,t.info.w,d.y)),i32(t.info.x),0.);}
fn surface(i:Vary,front:bool)->vec4f {
 let m=materials[i.material];let flags=u32(m.policy.y);
 let authored_front=front!=(i.mirrored!=0u);
 if((flags&4u)==0u && authored_front!=(camera.clip.w>0.)){discard;}
 var uv=i.uv;if((flags&128u)!=0u){uv.y=1.-uv.y;}
 var c=tex(m.maps.x,uv);var e=tex(m.maps.y,uv);
 if(m.imported[5].w>0.5){c=physical(m,0u,i);e=physical(m,1u,i);}
 var alpha=m.extra.z*c.a;var base=m.base.rgb*decode(c.rgb);
 if((flags&16u)!=0u){base*=i.color.rgb;alpha*=i.color.a;}
 if(alpha< m.policy.z || alpha<=0.){discard;}
 var n=normalize(i.normal);if(dot(n,camera.eye.xyz-i.world)<0.){n=-n;}
 var lit=camera.ambient.rgb;
 for(var j=0u;j<u32(camera.ambient.w);j++) {let l=lights[j];var d=l.pos.xyz-i.world;var scale=1.;if(l.pos.w==3.){d=l.pos.xyz;}else{let distance=max(dot(d,d),0.000001);scale=select(max(l.u.x*l.u.x,0.0001),max(l.u.w,0.0001),l.pos.w==0.)/distance;}let direction=normalize(d);if(l.pos.w==2.){scale*=smoothstep(l.v.w,l.radiance.w,dot(direction,l.v.xyz));}lit+=l.radiance.rgb*scale*max(dot(n,direction),0.);}
 var color=base*lit+m.emit.rgb*decode(e.rgb);if((flags&8u)!=0u){color=m.emit.rgb*decode(e.rgb);if((flags&16u)!=0u){color*=i.color.rgb;}}
 return vec4f(encode(max(color*camera.eye.w,vec3f(0.)))*alpha,alpha);
}
@fragment fn fs(i:Vary,@builtin(front_facing) front:bool)->@location(0) vec4f {return surface(i,front);}
@fragment fn depth(i:Vary,@builtin(front_facing) front:bool) {let unused=surface(i,front);}

@vertex fn sky_vs(@builtin(vertex_index) index:u32)->@builtin(position) vec4f {let p=array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.));return vec4f(p[index],1.,1.);}
@fragment fn sky_fs()->@location(0) vec4f {return camera.backdrop;}
