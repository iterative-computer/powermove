//! Shared panel/timeline dot language. Coordinates and radii are logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot { pub center: [f32; 2], pub radius: f32, pub alpha: f32, pub color: u32 }
pub const REFINE_SECONDS: f32 = 0.82;
pub const RESOLVE_SECONDS: f32 = 0.7;
pub fn detail(age: f32) -> f32 {
    if age <= 0.4 { return 0.7 * (age / 0.4).clamp(0.,1.); }
    0.7 + 3.3 * ((age - 0.4) / (REFINE_SECONDS - 0.4)).clamp(0.,1.).powf(1.6)
}
pub fn resolve(age: f32) -> f32 {
    let p = (age / RESOLVE_SECONDS).clamp(0., 1.);
    p * p * (3. - 2. * p)
}
pub fn hash(p: [f32; 2]) -> f32 { ((p[0]*12.9898+p[1]*78.233).sin()*43758.5453).rem_euclid(1.) }
pub fn generation_visibility(detail: f32, generation: usize, center: [f32; 2]) -> f32 {
    ((detail-generation as f32-hash(center)*0.85)/0.15).clamp(0.,1.)
}
/// Sample premultiplied RGBA at nine points in each dot's own neighbourhood.
/// Existing grid points belong only to their coarsest generation, never move or shrink.
pub fn matrix(size: [f32; 2], detail: f32, sample: impl Fn([f32; 2]) -> [f32; 4]) -> Vec<Dot> {
    let mut dots=Vec::new();
    if size.iter().any(|n| !n.is_finite() || *n <= 0.) { return dots; }
    for k in 0..4 {
        let pitch=16. / 2_f32.powi(k as i32);
        for y in 0..=(size[1].min(4096.)/pitch) as usize {
            for x in 0..=(size[0].min(4096.)/pitch) as usize {
                if k>0 && x%2==0 && y%2==0 { continue; }
                let center=[x as f32*pitch,y as f32*pitch];
                let vis=generation_visibility(detail,k,center);
                if vis<=0. { continue; }
                let mut s=[0.;4];
                for dy in -1..=1 { for dx in -1..=1 {
                    let p=[(center[0]+dx as f32*pitch*0.33).clamp(0.,size[0]),(center[1]+dy as f32*pitch*0.33).clamp(0.,size[1])];
                    let v=sample(p); for i in 0..4 { s[i]+=v[i]/9.; }
                }}
                let a=((s[3]-0.04)/0.31).clamp(0.,1.); let a=a*a*(3.-2.*a)*vis;
                if a>0. { let c=|i| (s[i]/s[3].max(0.001)*255.).clamp(0.,255.) as u32;
                    dots.push(Dot{center,radius:1.35,alpha:a,color:(c(0)<<16)|(c(1)<<8)|c(2)}); }
            }
        }
    }
    dots
}
fn noise(p: [f32;2]) -> f32 {
    let i=[p[0].floor(),p[1].floor()]; let f=[p[0]-i[0],p[1]-i[1]];
    let u=[f[0]*f[0]*(3.-2.*f[0]),f[1]*f[1]*(3.-2.*f[1])];
    let mix=|a:f32,b:f32,t:f32| a+(b-a)*t;
    mix(mix(hash(i),hash([i[0]+1.,i[1]]),u[0]),mix(hash([i[0],i[1]+1.]),hash([i[0]+1.,i[1]+1.]),u[0]),u[1])
}
fn fbm(mut p:[f32;2])->f32 { let(mut v,mut a)=(0.,0.5); for _ in 0..4 {v+=a*noise(p);p=[p[0]*2.03+11.7,p[1]*2.03+11.7];a*=0.5;} v }
/// Four drifting curtains, visible only through a fixed 3pt dot matrix.
/// Gold/ember/rose are derived from the theme accent and text, never fixed RGB.
pub fn aurora(size:[f32;2],age:f32,accent:u32,text:u32,muted:u32)->Vec<Dot> {
    let mut dots=Vec::new();
    if size.iter().any(|n|!n.is_finite()||*n<=0.) {return dots;}
    let (l,c,h)=super::color::to_oklch(accent);
    let gold=super::mix(super::color::oklch_fit((l+0.16).min(0.92),c,h+35.),text,0.12);
    let rose=super::mix(super::color::oklch_fit(l,c,h-35.),muted,0.12);
    for y in (1..size[1].min(4096.) as usize).step_by(3) { for x in (1..size[0].min(4096.) as usize).step_by(3) {
        let xn=x as f32/size[0]*2.2; let up=1.-y as f32/size[1]; let(mut swirl,mut rise)=(0_f32,0_f32);
        for j in 0..4 {let f=j as f32;
            let hem=0.04+0.25*f+0.09*(xn*(1.6+0.7*f)+age*(0.45+0.2*f)+f*2.1).sin()+0.09*(fbm([xn*1.4+age*0.12,f*3.7])-0.5);
            let above=up-hem;
            let curtain=if above<0. {(above*65.).exp()} else {0.3*(-above*3.).exp()+0.7*(-above*40.).exp()};
            let r=((fbm([xn*20.+age*0.2+f*5.,f*7.+up*0.025-age*0.02])-0.22)/0.5).clamp(0.,1.);
            let v=2.2*curtain*(0.08+0.92*r*r*(3.-2.*r))*(0.8+0.2*(xn*11.-age*2.2+f*1.3).sin())*(1.-0.1*f);
            if v>swirl {swirl=v;rise=(above*4.).clamp(0.,1.);}
        }
        let s=swirl.clamp(0.,1.);
        let color=super::mix(super::mix(gold,accent,(rise/0.35).clamp(0.,1.)),rose,((rise-0.45)/0.55).clamp(0.,1.)*0.45);
        dots.push(Dot{center:[x as f32,y as f32],radius:0.7,alpha:(0.02+0.1*s+0.88*s.powi(4)).min(1.),color});
    }} dots
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn generations_only_add_fixed_dots() {
        let sample=|_|[1.;4]; let coarse=matrix([64.,32.],1.,sample); let fine=matrix([64.,32.],4.,sample);
        assert!(coarse.len()<fine.len());
        for d in coarse {assert!(fine.iter().any(|f| f.center==d.center && f.radius==d.radius && f.alpha>=d.alpha));}
        assert!(fine.iter().all(|d|d.radius==1.35));
    }
    #[test] fn thresholds_and_stagger() {
        for k in 0..4 { assert_eq!(generation_visibility(k as f32,k,[8.,8.]),0.); assert_eq!(generation_visibility(k as f32+1.,k,[8.,8.]),1.); }
        assert!(detail(0.41)<1.); assert_eq!(detail(0.82),4.);
        assert_eq!(resolve(0.),0.); assert_eq!(resolve(0.7),1.);
    }
    #[test] fn empty_coverage_has_no_dots() {assert!(matrix([100.,40.],4., |_|[0.;4]).is_empty());}
    #[test] fn field_is_deterministic_and_drifting() {
        let a=aurora([60.,40.],1.,0xff8800,0xffffff,0x888888);
        assert_eq!(a,aurora([60.,40.],1.,0xff8800,0xffffff,0x888888));
        assert_ne!(a,aurora([60.,40.],2.,0xff8800,0xffffff,0x888888));
        assert!(a.iter().all(|d|d.radius==0.7));
    }
}
