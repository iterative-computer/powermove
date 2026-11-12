//! Developer-only synchronous stage timing. No window, app or Blender dependency.
use pm_studio::{gpu::Renderer, scene, oidn::{Denoiser,Quality},environment::AdaptiveSampling};
use std::{cell::Cell,time::Instant};
fn main()->Result<(),String>{
    let args:Vec<_>=std::env::args().collect();
    let value=|name:&str,default:&str|args.windows(2).find(|p|p[0]==name).map(|p|p[1].clone()).unwrap_or_else(||default.into());
    let scenes=value("--scenes","hero,chrome");let presets=value("--presets","preview,production");
    let width=value("--width","1920").parse::<u32>().map_err(|e|e.to_string())?;
    let height=value("--height","1080").parse::<u32>().map_err(|e|e.to_string())?;
    let start=Instant::now();let mut renderer=Renderer::new(width,height)?;let init_ms=start.elapsed().as_secs_f64()*1000.;
    let denoiser=Denoiser::new()?;
    // Per-preset native guide policy is selected below.
    for name in scenes.split(',') {
        let setup=Instant::now();
        let scene=scene::preset(name,"starlight",0.).ok_or("unknown scene")?;
        let prepared=scene.prepare();
        let scene_prepare_ms=setup.elapsed().as_secs_f64()*1000.;
        let upload=Instant::now();renderer.set_scene(&scene,&prepared);
        let scene_upload_ms=upload.elapsed().as_secs_f64()*1000.;
        renderer.set_screen(&scene::screen_wallpaper(512,744,0.),512,744);
        renderer.trace(1,8,true,64);renderer.post_only(0);
        for preset in presets.split(',') {
            let (samples,min_samples,noise,bounces,quality)=match preset {
                "draft"=>(16,4,0.12,4,Quality::Balanced),
                "preview"=>(64,16,0.06,6,Quality::Balanced),
                "production"=>(128,8,0.12,8,Quality::Balanced),
                "final"=>(1024,512,0.015,8,Quality::High),
                "refine"=>(32,32,0.,6,Quality::Balanced),
                _=>return Err("unknown preset".into()),
            };
            renderer.sampling_policy=if preset=="production" {pm_studio::sampling::SamplingPolicy::PRODUCTION}else{Default::default()};
            renderer.set_adaptive_sampling_with_floor(AdaptiveSampling{enabled:noise>0.,min_samples,relative_error:noise,absolute_error:0.001},true);
            renderer.caustics_enabled = !matches!(preset,"draft"|"preview") && value("--caustics","on")!="off";
            renderer.set_precomputed_caustics(false);
            renderer.set_precomputed_caustics(matches!(preset,"production"|"final"));
            let half_guides=match value("--guide-scale","auto").as_str() {"auto"=>preset!="final","2"=>true,"1"=>false,_=>return Err("guide scale must be auto, 1 or 2".into())};
            denoiser.set_half_resolution_guides(half_guides);
            denoiser.set_quality(match value("--oidn-quality","auto").as_str() {"auto"=>quality,"fast"=>Quality::Fast,"balanced"=>Quality::Balanced,"high"=>Quality::High,_=>return Err("unknown OIDN quality".into())});
            let repeats=value("--repeat","1").parse::<u32>().map_err(|e|e.to_string())?;
            if !(1..=100).contains(&repeats){return Err("repeat must be 1..100".into());}
            for run in 0..repeats {
            let frame=Instant::now();
            // Match native stills: converge to the noise target/sample ceiling, no time cap.
            let samples=value("--samples",&samples.to_string()).parse::<u32>().map_err(|e|e.to_string())?;
            let bounces=value("--bounces",&bounces.to_string()).parse::<u32>().map_err(|e|e.to_string())?;
            renderer.trace(samples,bounces,true,64);
            let filter=Cell::new(Default::default());
            let denoise=value("--denoise",if preset=="draft" {"guided"}else{"oidn"});
            if denoise=="guided" {renderer.prepare_frame(4,None)?;}
            else if denoise=="none" {renderer.prepare_frame(0,None)?;}
            else if denoise=="oidn" {renderer.prepare_frame(0,Some(&|color,shade,albedo,normal|{
                denoiser.run_layers_interruptible(&mut [color,shade],albedo,normal,4,width as usize,height as usize,None)?;
                filter.set(denoiser.timings());Ok(())
            }))?;}else{return Err("unknown denoiser".into());}
            let ready_ms=frame.elapsed().as_secs_f64()*1000.;
            let readback=Instant::now();let (pixels,_)=renderer.frame_pixels();let output_readback_ms=readback.elapsed().as_secs_f64()*1000.;
            let encode=Instant::now();let mut png=Vec::new();
            {
                let mut encoder=png::Encoder::new(&mut png,width,height);encoder.set_color(png::ColorType::Rgba);encoder.set_depth(png::BitDepth::Eight);encoder.set_compression(png::Compression::Fast);
                encoder.write_header().map_err(|e|e.to_string())?.write_image_data(&pixels).map_err(|e|e.to_string())?;
            }
            let encode_ms=encode.elapsed().as_secs_f64()*1000.;
            let frame_ms=frame.elapsed().as_secs_f64()*1000.;
            let mut counts=renderer.sample_counts();counts.sort_by(f32::total_cmp);
            println!("{}",serde_json::json!({"scene":name,"preset":preset,"run":run,"width":width,"height":height,"hardware_rt":renderer.uses_hardware_rt(),"samples":renderer.samples,"init_ms":init_ms,"scene_prepare_ms":scene_prepare_ms,"scene_upload_ms":scene_upload_ms,"sample_mean":counts.iter().map(|&n|n as f64).sum::<f64>()/counts.len() as f64,"sample_p50":counts[counts.len()/2],"sample_p95":counts[counts.len()*95/100],"pipeline":renderer.timings,"oidn":filter.get(),"output_readback_ms":output_readback_ms,"png_encode_ms":encode_ms,"frame_ms":frame_ms,"half_resolution_guides":half_guides,"ready_ms":ready_ms, "png_compression":"fast", "caustics":renderer.caustics_enabled,"denoiser":denoise, "convergence_time_cap_ms":0,"png_bytes":png.len()}));
            if let Some(path)=args.windows(2).find(|a|a[0]=="--out").map(|a|std::path::Path::new(&a[1])) {std::fs::create_dir_all(path).map_err(|e|e.to_string())?;std::fs::write(path.join(format!("{name}-{preset}-{run}.png")),&png).map_err(|e|e.to_string())?;}
            }
        }
    }
    Ok(())
}
