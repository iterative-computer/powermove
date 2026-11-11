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
    for name in scenes.split(',') {
        let scene=scene::preset(name,"starlight",0.).ok_or("unknown scene")?;
        renderer.set_scene(&scene,&scene.prepare());
        renderer.set_screen(&scene::screen_wallpaper(512,744,0.),512,744);
        renderer.trace(1,8,true,64);renderer.post_only(0);
        for preset in presets.split(',') {
            let (samples,min_samples,noise,bounces,budget,quality)=match preset {
                "preview"=>(64,16,0.06,6,2000,Quality::Fast),
                "production"=>(256,32,0.03,8,3000,Quality::Balanced),
                "final"=>(1024,512,0.015,8,0,Quality::High),
                "refine"=>(32,32,0.,6,0,Quality::High),
                _=>return Err("unknown preset".into()),
            };
            renderer.set_adaptive_sampling(AdaptiveSampling{enabled:noise>0.,min_samples,relative_error:noise,absolute_error:0.001});
            renderer.set_precomputed_caustics(preset!="preview");denoiser.set_quality(quality);
            let frame=Instant::now();
            renderer.trace_budgeted_interruptible(samples,bounces,true,64,min_samples,budget,&||false);
            let filter=Cell::new(Default::default());
            renderer.prepare_frame(0,Some(&|color,shade,albedo,normal|{
                denoiser.run_layers_interruptible(&mut [color,shade],albedo,normal,4,width as usize,height as usize,None)?;
                filter.set(denoiser.timings());Ok(())
            }))?;
            let readback=Instant::now();let (pixels,_)=renderer.frame_pixels();let output_readback_ms=readback.elapsed().as_secs_f64()*1000.;
            let encode=Instant::now();let mut png=Vec::new();
            {
                let mut encoder=png::Encoder::new(&mut png,width,height);encoder.set_color(png::ColorType::Rgba);encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header().map_err(|e|e.to_string())?.write_image_data(&pixels).map_err(|e|e.to_string())?;
            }
            let encode_ms=encode.elapsed().as_secs_f64()*1000.;
            println!("{}",serde_json::json!({"scene":name,"preset":preset,"width":width,"height":height,"hardware_rt":renderer.uses_hardware_rt(),"samples":renderer.samples,"init_ms":init_ms,"pipeline":renderer.timings,"oidn":filter.get(),"output_readback_ms":output_readback_ms,"png_encode_ms":encode_ms,"frame_ms":frame.elapsed().as_secs_f64()*1000.,"png_bytes":png.len()}));
        }
    }
    Ok(())
}
