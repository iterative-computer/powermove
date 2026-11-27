//! Opaque shadow shortcuts agree with forced full material evaluation.
use pm_studio::{gpu::Renderer,scene,math::Vec3};
#[test]
fn unmapped_opaque_casters_match_white_alpha_map_fallback() {
    let (w,h)=(96,64);let mut r=Renderer::new(w,h).unwrap();
    for hardware in [false,true] {
        r.hardware_rt=hardware;
        let mut s=scene::preset("chrome","starlight",0.).unwrap();
        // Avoid a photon map making the shadow parity test needlessly costly.
        r.caustics_enabled=false;
        for material in &mut s.materials {material.transmission=0.;}
        let mut p=s.prepare();r.set_scene(&s,&p);r.set_screen(&[255;4],1,1);
        r.trace(128,6,true,64);let expected=r.finish(0).0;
        // A real map forces the slow opacity path but changes no colour/alpha.
        for material in &mut p.materials {material.maps[0]=0.;}
        r.set_scene(&s,&p);r.trace(128,6,true,64);let mapped=r.finish(0).0;
        let rms=(expected.iter().zip(&mapped).map(|(a,b)|(*a as f64-*b as f64).powi(2)).sum::<f64>()/expected.len() as f64).sqrt();
        assert!(rms<0.5,"shadow map parity hardware={hardware}: {rms} code values");
        // Alpha is still evaluated: a transparent mapped caster cannot block.
        let mut transparent=s.clone();
        for material in &mut transparent.materials {material.base=Vec3::splat(1.);}
        let mut p=transparent.prepare();for material in &mut p.materials {material.maps[0]=0.;}
        r.set_scene(&transparent,&p);r.set_screen(&[255,255,255,0],1,1);
        r.trace(128,6,true,64);let clear=r.finish(0).0;
        assert_ne!(clear,expected,"mapped alpha must survive shortcut");
    }
}
