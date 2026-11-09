//! Environment lighting and participating media. All GPU data is a versioned
//! trailer after the existing light/instance rows; old uploads need no changes.
use crate::{math::{Vec3, v3}, scene::GpuLight};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct World {
    pub sky: Option<Sky>,
    /// Linear scene-referred equirectangular pixels. Decode files before upload.
    pub hdri: Option<EnvironmentMap>,
    pub atmosphere: Atmosphere,
    pub volumes: Vec<Volume>,
    pub adaptive: AdaptiveSampling,
}
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Sky {
    pub elevation: f32,
    pub azimuth: f32,
    pub turbidity: f32,
    pub ground_albedo: Vec3,
    pub intensity: f32,
    pub sun_intensity: f32,
    /// Angular radius, degrees (the real sun is about 0.266 degrees).
    pub sun_radius: f32,
    pub overcast: f32,
    pub visible: bool,
}
impl Default for Sky {
    fn default() -> Self { Self { elevation: 45., azimuth: 0., turbidity: 2., ground_albedo: Vec3::splat(0.2), intensity: 1., sun_intensity: 1., sun_radius: 0.266, overcast: 0., visible: true } }
}
impl Sky {
    pub fn preset(name: &str) -> Option<Self> {
        Some(match name {
            "clear-noon" => Self { elevation: 65., ..Self::default() },
            "golden-hour" => Self { elevation: 5., azimuth: -65., turbidity: 3., ..Self::default() },
            "overcast" => Self { elevation: 35., overcast: 1., sun_intensity: 0., ..Self::default() },
            "blue-hour" => Self { elevation: -5., sun_intensity: 0., ..Self::default() },
            "night" => Self { elevation: -18., sun_intensity: 0., ..Self::default() },
            _ => return None,
        })
    }
    pub fn direction(&self) -> Vec3 {
        let (e,a) = (self.elevation.to_radians(),self.azimuth.to_radians());
        v3(a.sin()*e.cos(), e.sin(), a.cos()*e.cos())
    }
    pub fn sun(&self) -> Option<GpuLight> {
        if self.elevation <= -0.8 || self.sun_intensity <= 0. { return None; }
        let d = self.direction();
        let tr = sun_transmittance(v3(0., EARTH+2., 0.), d, self.turbidity);
        let omega = 2.*PI*(1.-self.sun_radius.to_radians().cos());
        Some(GpuLight::sun(d,self.sun_radius.to_radians(), tr*(20.*self.intensity*self.sun_intensity*(1.-self.overcast)/omega.max(1e-8))))
    }
    /// A ground observer's single-scattering Rayleigh/Mie sky, integrated in a
    /// spherical atmosphere. A bounded diffuse closure approximates higher orders;
    /// it is not the full Bruneton multiple-scattering algorithm.
    pub fn radiance(&self, d: Vec3) -> Vec3 {
        if d.y < 0. { return self.ground_albedo * (0.06 + 0.24*self.direction().y.max(0.)) * self.intensity; }
        let ro = v3(0.,EARTH+2.,0.);
        let sun = self.direction();
        let distance = atmosphere_exit(ro,d);
        let ds = distance/24.;
        let mu = d.dot(sun);
        let pr = 3.*(1.+mu*mu)/(16.*PI);
        let pm = hg(mu,0.76);
        let br = v3(5.8e-6,13.5e-6,33.1e-6);
        let bm = 2e-6 * self.turbidity.max(1.);
        let mut odr = 0.; let mut odm = 0.; let mut out = Vec3::ZERO;
        for i in 0..24 {
            let p = ro+d*((i as f32+0.5)*ds);
            let h = (p.length()-EARTH).max(0.);
            let dr = (-h/8000.).exp(); let dm = (-h/1200.).exp();
            odr += dr*ds*0.5; odm += dm*ds*0.5;
            let eye_tr = exp3(-(br*odr+Vec3::splat(bm*1.1*odm)));
            let sun_tr = sun_transmittance(p,sun,self.turbidity);
            out = out + mul3(mul3(eye_tr,sun_tr),br*(dr*pr)+Vec3::splat(bm*dm*pm))*ds*20.;
            odr += dr*ds*0.5; odm += dm*ds*0.5;
        }
        let day = ((sun.y+0.12)/0.25).clamp(0.,1.);
        let diffuse = v3(0.018,0.035,0.075)*day*(1.+self.ground_albedo.dot(Vec3::splat(1./3.)));
        let night = v3(0.0002,0.00035,0.0008);
        let clear = out+diffuse+night;
        // CIE-style overcast luminance distribution, an artistic cloud closure.
        let cloud = Vec3::splat((0.15+0.6*day)*(1.+2.*d.y)/3.);
        (clear*(1.-self.overcast)+cloud*self.overcast)*self.intensity
    }
    pub fn bake(&self, width:u32, height:u32) -> EnvironmentMap {
        let pixels = (0..height).flat_map(|y| (0..width).map(move |x| {
            let theta=PI*(y as f32+0.5)/height as f32;
            let phi=TAU*(x as f32+0.5)/width as f32;
            self.radiance(v3(theta.sin()*phi.sin(),theta.cos(),theta.sin()*phi.cos()))
        })).collect();
        EnvironmentMap { width,height,pixels,visible:self.visible,..EnvironmentMap::default() }
    }
}
const EARTH:f32=6_360_000.;
fn atmosphere_exit(ro:Vec3,d:Vec3)->f32 {
    let b=ro.dot(d); let c=ro.dot(ro)-(EARTH+80_000.).powi(2);
    (-b+(b*b-c).max(0.).sqrt()).max(0.)
}
fn sun_transmittance(p:Vec3,d:Vec3,turbidity:f32)->Vec3 {
    let b=p.dot(d); let c=p.dot(p)-EARTH*EARTH;
    if b<0. && b*b>c { return Vec3::ZERO; }
    let ds=atmosphere_exit(p,d)/12.; let mut r=0.;let mut m=0.;
    for i in 0..12 {
        let h=((p+d*((i as f32+0.5)*ds)).length()-EARTH).max(0.);
        r+=(-h/8000.).exp()*ds;m+=(-h/1200.).exp()*ds;
    }
    exp3(-(v3(5.8e-6,13.5e-6,33.1e-6)*r+Vec3::splat(2e-6*turbidity.max(1.)*1.1*m)))
}
fn exp3(v:Vec3)->Vec3 { v3(v.x.exp(),v.y.exp(),v.z.exp()) }
fn mul3(a:Vec3,b:Vec3)->Vec3 { v3(a.x*b.x,a.y*b.y,a.z*b.z) }
pub fn hg(cos_theta:f32,g:f32)->f32 { (1.-g*g)/(4.*PI*(1.+g*g-2.*g*cos_theta).max(1e-8).powf(1.5)) }

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EnvironmentMap {
    pub width:u32,
    pub height:u32,
    pub pixels:Vec<Vec3>,
    pub rotation:f32,
    pub intensity:f32,
    pub visible:bool,
}
impl Default for EnvironmentMap {
    fn default()->Self { Self { width:0,height:0,pixels:vec![],rotation:0.,intensity:1.,visible:true } }
}
impl EnvironmentMap {
    /// HDR/RGBE, EXR and grayscale/PNG sources use the same linear float contract.
    pub fn load(path:impl AsRef<std::path::Path>)->Result<Self,String> {
        let image=image::ImageReader::open(path).map_err(|e|e.to_string())?.decode().map_err(|e|e.to_string())?.to_rgb32f();
        let (width,height)=image.dimensions();
        let map=Self {width,height,pixels:image.pixels().map(|p|v3(p[0],p[1],p[2])).collect(),..Self::default()};
        map.validate()?; Ok(map)
    }
    pub fn validate(&self)->Result<(),String> {
        if self.width==0 || self.height==0 || self.width>4096 || self.height>2048 || self.pixels.len()!=self.width as usize*self.height as usize { return Err("environment map must contain width × height pixels, at most 4096 × 2048".into()); }
        if !self.rotation.is_finite() || !self.intensity.is_finite() || self.intensity<0. || self.pixels.iter().any(|p|p.to_array().iter().any(|v|!v.is_finite()||*v<0.)) { return Err("environment pixels and intensity must be finite and nonnegative".into()); } Ok(())
    }
    pub fn solid_angle(&self,i:usize)->f64 {
        let y=i/self.width as usize;
        let a=std::f64::consts::PI*y as f64/self.height as f64;
        let b=std::f64::consts::PI*(y+1) as f64/self.height as f64;
        std::f64::consts::TAU/self.width as f64*(a.cos()-b.cos())
    }
    /// CDF is accumulated in f64; a small uniform mixture preserves support in
    /// very dark texels. Sampling is uniform in solid angle within each texel.
    pub fn distribution(&self)->Vec<[f32;4]> {
        let weights:Vec<f64>=self.pixels.iter().enumerate().map(|(i,p)| (p.dot(v3(0.2126,0.7152,0.0722)) as f64+1e-6)*self.solid_angle(i)).collect();
        let total=weights.iter().sum::<f64>().max(1e-30); let mut cdf=0.;
        weights.iter().enumerate().map(|(i,w)| { cdf+=w/total;[(if i+1==weights.len(){1.}else{cdf}) as f32, (w/total/self.solid_angle(i)) as f32,0.,0.] }).collect()
    }
}
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Atmosphere {
    /// Extinction per scene unit at base_height. Zero disables global fog.
    pub density:f32,
    pub color:Vec3,
    pub anisotropy:f32,
    pub base_height:f32,
    pub height_falloff:f32,
    /// Finite atmosphere distance, in scene units, also used for miss rays.
    pub distance:f32,
}
impl Default for Atmosphere { fn default()->Self { Self {density:0.,color:Vec3::splat(0.9),anisotropy:0.,base_height:0.,height_falloff:0.,distance:10000.} } }
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all="snake_case")]
pub enum VolumeShape { Box, Sphere }
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Volume {
    pub shape:VolumeShape,
    pub center:Vec3,
    /// Box half extents; for spheres, size.x is the radius.
    pub size:Vec3,
    pub density:f32,
    /// Single-scattering albedo (0..1).
    pub color:Vec3,
    pub anisotropy:f32,
}
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AdaptiveSampling { pub enabled:bool,pub min_samples:u32,pub relative_error:f32,pub absolute_error:f32 }
impl Default for AdaptiveSampling { fn default()->Self { Self {enabled:false,min_samples:64,relative_error:0.025,absolute_error:0.001} } }
impl World {
    pub fn validate(&self)->Result<(),String> {
        if let Some(m)=&self.hdri {m.validate()?;}
        if let Some(s)=self.sky {
            if [s.elevation,s.azimuth,s.turbidity,s.intensity,s.sun_intensity,s.sun_radius,s.overcast].iter().any(|x|!x.is_finite()) || !(-90. ..=90.).contains(&s.elevation)||!(1. ..=10.).contains(&s.turbidity)||!(0.01..=10.).contains(&s.sun_radius)||s.intensity<0.||s.sun_intensity<0.||!(0. ..=1.).contains(&s.overcast) {return Err("invalid sky parameters".into());}
        }
        let a=self.atmosphere;
        if self.volumes.len()>32 || !a.density.is_finite() || a.density<0. || !a.height_falloff.is_finite() || a.height_falloff<0. || !a.distance.is_finite() || a.distance<=0. || !a.base_height.is_finite() || !a.anisotropy.is_finite() || a.anisotropy.abs()>0.95 || !color_valid(a.color) {return Err("invalid atmosphere (maximum 32 local volumes)".into());}
        for v in &self.volumes {
            if !v.density.is_finite() || v.density<0. || !v.anisotropy.is_finite() || v.anisotropy.abs()>0.95 || !color_valid(v.color)||v.size.to_array().iter().any(|x|!x.is_finite()||*x<=0.)||v.center.to_array().iter().any(|x|!x.is_finite()) {return Err("invalid fog volume".into());}
        }
        if self.adaptive.min_samples<4 || !self.adaptive.relative_error.is_finite() || self.adaptive.relative_error<0. || !self.adaptive.absolute_error.is_finite() || self.adaptive.absolute_error<0. {return Err("invalid adaptive sampling".into());} Ok(())
    }
    pub fn sun(&self)->Option<GpuLight> { self.sky.as_ref().and_then(Sky::sun) }
    /// Trailer version 1. Offsets are relative to the trailer, measured in vec4s.
    /// Header: magic/version/mapW/mapH; map rotation/intensity/visible/mapOffset;
    /// fog color/density; g/base/falloff/distance; volumesOffset/count/lightCDF/0;
    /// adaptive enabled/min/relative/absolute. Then pixels, CDF, volumes, lights.
    pub fn pack(&self,lights:&[GpuLight])->Vec<[f32;4]> {
        let baked=self.sky.map(|s|s.bake(128,64));
        let map=self.hdri.as_ref().or(baked.as_ref());
        let a=self.atmosphere;
        let mut data=vec![[0.;4];6];
        data[0]=[735_031.,1.,map.map_or(0.,|m|m.width as f32),map.map_or(0.,|m|m.height as f32)];
        data[1]=[map.map_or(0.,|m|m.rotation.to_radians()),map.map_or(1.,|m|m.intensity),map.map_or(0.,|m|u32::from(m.visible) as f32),6.];
        data[2]=a.color.to4(a.density);data[3]=[a.anisotropy,a.base_height,a.height_falloff,a.distance];
        if let Some(m)=map {data.extend(m.pixels.iter().map(|p|p.to4(0.)));data.extend(m.distribution());}
        data[4]=[data.len() as f32,self.volumes.len() as f32,0.,0.];
        for v in &self.volumes {data.extend([v.center.to4(match v.shape{VolumeShape::Box=>0.,VolumeShape::Sphere=>1.}),v.size.to4(v.density),v.color.to4(v.anisotropy)]);}
        data[4][2]=data.len() as f32;
        let weights:Vec<f64>=lights.iter().map(|l|light_power(l).max(1e-12)).collect();
        let sum=weights.iter().sum::<f64>().max(1e-30);let mut cdf=0.;
        for (i,w) in weights.iter().enumerate() {cdf+=w/sum;data.push([if i+1==weights.len(){1.}else{cdf as f32},(w/sum) as f32,0.,0.]);}
        let cdf=data[4][2] as usize;
        append_light_tree(&mut data,lights,cdf);
        let s=self.adaptive; data[5]=[u32::from(s.enabled) as f32,s.min_samples as f32,s.relative_error,s.absolute_error];data
    }
}
fn color_valid(p:Vec3)->bool {p.to_array().iter().all(|v|v.is_finite()&&(0. ..=1.).contains(v))}
pub fn light_power(l:&GpuLight)->f64 {
    let lum=(l.radiance[0]*0.2126+l.radiance[1]*0.7152+l.radiance[2]*0.0722).max(0.) as f64;
    let area=match l.pos[3] as u32 {0=>l.u[3].abs(),3=>TAU*(1.-l.u[0]),_=>4.*PI*l.u[0]*l.u[0]};lum*area as f64
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn sky_is_finite_nonnegative_and_sun_is_small() {
        for name in ["clear-noon","golden-hour","overcast","blue-hour","night"] {
            let s=Sky::preset(name).unwrap();let map=s.bake(32,16);map.validate().unwrap();
            if let Some(l)=s.sun(){assert!(l.u[0]>0.9999);assert!(l.radiance.iter().all(|x|x.is_finite()));}
            assert!(map.pixels.iter().any(|p|p.length()>0.));
        }
        let s=Sky::default();let c=s.radiance(Vec3::Y);assert!(c.z>c.x,"Rayleigh sky must be blue: {c:?}");
    }
    #[test] fn map_pdf_integrates_to_one_including_poles() {
        let mut m=EnvironmentMap{width:16,height:8,pixels:vec![Vec3::splat(1.);128],..Default::default()};
        m.pixels[7]=Vec3::splat(10000.);
        let d=m.distribution();let integral:f64=d.iter().enumerate().map(|(i,p)|p[1] as f64*m.solid_angle(i)).sum();
        assert!((integral-1.).abs()<1e-6);assert_eq!(d.last().unwrap()[0],1.);
        assert!((m.pixels.iter().enumerate().map(|(i,_)|m.solid_angle(i)).sum::<f64>()-4.*std::f64::consts::PI).abs()<1e-12);
    }
    #[test] fn hg_normalizes_and_has_expected_forward_mean() {
        for g in [-0.8,0.,0.8] {let n=100000;let mut sum=0.;let mut mean=0.;for i in 0..n {let mu=-1.+2.*(i as f32+0.5)/n as f32;let p=hg(mu,g)*4.*PI/n as f32;sum+=p;mean+=p*mu;}assert!((sum-1.).abs()<0.001);assert!((mean-g).abs()<0.001);}
    }
    #[test] fn light_distribution_preserves_power_ratios() {
        let a=GpuLight::sphere(Vec3::ZERO,1.,Vec3::splat(1.));let b=GpuLight::sphere(Vec3::ZERO,1.,Vec3::splat(9.));let p=World::default().pack(&[a,b]);let at=p[4][2] as usize;assert!((p[at][1]-0.1).abs()<1e-5);assert_eq!(p[at+1][0],1.);
    }
    #[test] fn old_world_defaults_roundtrip_and_rejects_invalid_data() {
        let w:World=serde_json::from_str("{}").unwrap();w.validate().unwrap();assert_eq!(w.pack(&[])[0][0],735031.);
        let mut w=w;w.atmosphere.density=-1.;assert!(w.validate().is_err());
    }
}

/// Validate optional GPU trailers before accepting untrusted binary uploads.
pub fn validate_packed(data:&[[f32;4]], light_count:usize)->Result<(),String> {
    if data.is_empty(){return Ok(());}
    if data.len()<6 || data[0][0]!=735031. || data[0][1]!=1. {return Err("invalid environment trailer header".into());}
    let integer=|v:f32,max:usize|->Result<usize,String>{if !v.is_finite()||v<0.||v.fract()!=0.||v>max as f32 {Err("invalid environment trailer offset/count".into())}else{Ok(v as usize)}};
    let w=integer(data[0][2],4096)?;let h=integer(data[0][3],2048)?;
    if (w==0)!=(h==0){return Err("invalid environment dimensions".into());}
    let pixels=w*h;let volumes=integer(data[4][1],32)?;
    if data[1][3]!=6. || data[4][0]!= (6+pixels*2) as f32 || data[4][2]!=(6+pixels*2+volumes*3) as f32 || data.len()!=6+pixels*2+volumes*3+light_count+if data[4][3]>0.{3*(2*light_count).saturating_sub(1)}else{0} {return Err("invalid environment trailer ranges".into());}
    if data.iter().flatten().any(|x|!x.is_finite()) || data[2][3]<0. || data[3][2]<0. || data[3][3]<=0. || data[3][0].abs()>0.95 || !color_valid(v3(data[2][0],data[2][1],data[2][2])) {return Err("invalid environment trailer values".into());}
    for (base,count) in [(6+pixels,pixels),(data[4][2] as usize,light_count)] {
        let mut previous=0.;for i in 0..count {let p=data[base+i];if p[0]<previous||p[0]>1.||p[1]<0.{return Err("invalid environment CDF".into());}previous=p[0];}
        if count>0 && previous!=1.{return Err("environment CDF must end at one".into());}
    }
    if data[4][3]>0. {
        if light_count<=8 || data[4][3]!=(6+pixels*2+volumes*3+light_count) as f32 {return Err("invalid light tree offset".into());}
        let base=data[4][3] as usize;let nodes=light_count*2-1;
        for i in 0..nodes {
            let a=data[base+i*3];let b=data[base+i*3+1];let c=data[base+i*3+2];
            let parent=integer(c[2],nodes-1)?;if (i==0 && parent!=0)||(i>0 && parent>=i){return Err("cyclic light tree parent".into());}
            if c[0]<0.||c[1]<0.{return Err("negative light tree power".into());}
            if b[3]<0. {integer(a[3],light_count-1)?;}else{let l=integer(a[3],nodes-1)?;let r=integer(b[3],nodes-1)?;if l<=i||r<=i {return Err("cyclic light tree".into());}}
        }
    }
    for i in 0..volumes {
        let at=data[4][0] as usize+i*3;
        if data[at+1][3]<0. || data[at+1][..3].iter().any(|x|*x<=0.)||data[at+2][3].abs()>0.95||!color_valid(v3(data[at+2][0],data[at+2][1],data[at+2][2])) {return Err("invalid packed volume".into());}
    }
    Ok(())
}

/// Balanced spatial light hierarchy. Each node stores bounds, finite/infinite
/// power and parent. Leaf indices in the CDF allow O(log N) reverse PMF lookup.
fn append_light_tree(data:&mut Vec<[f32;4]>,lights:&[GpuLight],cdf:usize) {
    if lights.len()<=8{return;}
    let base=data.len();data[4][3]=base as f32;
    fn bounds(l:&GpuLight)->(Vec3,Vec3){
        let p=v3(l.pos[0],l.pos[1],l.pos[2]);
        if l.pos[3]==3. {return (Vec3::ZERO,Vec3::ZERO);}
        let r=if l.pos[3]==0. {v3(l.u[0].abs()+l.v[0].abs(),l.u[1].abs()+l.v[1].abs(),l.u[2].abs()+l.v[2].abs())}else{Vec3::splat(l.u[0])};(p-r,p+r)
    }
    fn build(data:&mut Vec<[f32;4]>,lights:&[GpuLight],indices:&mut[usize],base:usize,cdf:usize,parent:usize)->usize {
        let index=(data.len()-base)/3;data.extend([[0.;4];3]);
        let(mut lo,mut hi)=(Vec3::splat(f32::MAX),Vec3::splat(f32::MIN));let mut finite=0.;let mut infinite=0.;
        for &i in indices.iter(){let(a,b)=bounds(&lights[i]);lo=lo.min(a);hi=hi.max(b);let p=light_power(&lights[i]).max(1e-12);if lights[i].pos[3]==3.{infinite+=p;}else{finite+=p;}}
        let(left,right)=if indices.len()==1 {data[cdf+indices[0]][2]=index as f32;(indices[0] as f32,-1.)}else{
            let extent=hi-lo;let axis=if extent.x>=extent.y&&extent.x>=extent.z{0}else if extent.y>=extent.z{1}else{2};
            indices.sort_unstable_by(|&a,&b|lights[a].pos[axis].total_cmp(&lights[b].pos[axis]).then(a.cmp(&b)));
            let mid=indices.len()/2;let(a,b)=indices.split_at_mut(mid);let left=build(data,lights,a,base,cdf,index);let right=build(data,lights,b,base,cdf,index);(left as f32,right as f32)
        };
        let at=base+index*3;data[at]=lo.to4(left);data[at+1]=hi.to4(right);data[at+2]=[finite as f32,infinite as f32,parent as f32,0.];index
    }
    build(data,lights,&mut(0..lights.len()).collect::<Vec<_>>(),base,cdf,0);
}
