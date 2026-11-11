//! Native viewers reuse a resolved GPU image across compositor layer targets.
use pm_studio::{gpu::{Renderer,PresentTarget},scene::preset};
use std::cell::Cell;
fn target(r:&Renderer,w:u32,h:u32)->wgpu::Texture {
    r.device().create_texture(&wgpu::TextureDescriptor {
        label:Some("native surface regression"),size:wgpu::Extent3d{width:w,height:h,depth_or_array_layers:1},
        mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,
        usage:wgpu::TextureUsages::COPY_SRC|wgpu::TextureUsages::COPY_DST|wgpu::TextureUsages::RENDER_ATTACHMENT,view_formats:&[],
    })
}
fn read(r:&Renderer,t:&wgpu::Texture)->Vec<u8>{
    let stride=(t.width()*4).div_ceil(256)*256;
    let buffer=r.device().create_buffer(&wgpu::BufferDescriptor{label:None,size:u64::from(stride)*u64::from(t.height()),usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    let mut encoder=r.device().create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(t.as_image_copy(),wgpu::TexelCopyBufferInfo{buffer:&buffer,layout:wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(stride),rows_per_image:Some(t.height())}},t.size());
    r.queue().submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read,|r|r.unwrap());
    r.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped=buffer.slice(..).get_mapped_range().unwrap();
    mapped.chunks_exact(stride as usize).flat_map(|row|row[..t.width() as usize*4].iter().copied()).collect()
}
#[test]
fn prepared_layers_share_one_filter_and_preserve_their_masks(){
    let mut r=Renderer::new(128,72).unwrap();
    let scene=preset("chrome","blue",0.).unwrap();
    r.set_scene(&scene,&scene.prepare());r.set_void(None);r.trace(8,6,true,64);
    let calls=Cell::new(0);
    r.prepare_frame(0,Some(&|_,_,_,_|{calls.set(calls.get()+1);Ok(())})).unwrap();
    let mut images=vec![];
    for layer in [0,1]{
        let t=target(&r,128,72);let view=t.create_view(&Default::default());
        r.present_frame(PresentTarget{view:&view,format:t.format(),layer:Some(layer),include_void:false,flip_y:false});
        images.push(read(&r,&t));
    }
    assert_eq!(calls.get(),1,"guide/beauty filtering is once per frame");
    assert!(images.iter().all(|im|im.chunks_exact(4).any(|p|p[3]>0)));
    assert!(images[0].chunks_exact(4).zip(images[1].chunks_exact(4)).all(|(a,b)|a[3]==0||b[3]==0));
    assert_eq!(r.samples,8);
    assert_eq!(r.prepare_frame(0,Some(&|_,_,_,_|Err("denoiser failed".into()))).unwrap_err(),"denoiser failed");
}
#[cfg(all(feature="metalfx",target_os="macos"))]
#[test]
fn metalfx_in_process_texture_is_retained_and_flips_without_readback(){
    let mut r=Renderer::new(64,36).unwrap();let scene=preset("hero","blue",0.).unwrap();
    r.set_scene(&scene,&scene.prepare());
    assert!(r.queue_live_trace(3,6).is_err());
    assert_eq!(r.samples,0);
    r.live=true;r.queue_live_trace(3,6).unwrap();
    let (source,_)=r.finish_metalfx_texture(128,72,false,false).unwrap();
    let timings=r.timings;
    assert!(timings.trace_deferred);
    assert!(timings.trace_ms>0. && timings.guides_ms>0. && timings.metalfx_ms>0. && timings.post_ms>0.);
    assert_eq!(timings.denoise_ms,0.,"Live never invokes CPU OIDN");
    assert!(timings.metalfx_gpu_ms>0. && timings.metalfx_gpu_ms<=timings.metalfx_ms);
    let retained=target(&r,128,72);let mut encoder=r.device().create_command_encoder(&Default::default());
    encoder.copy_texture_to_texture(source.as_image_copy(),retained.as_image_copy(),source.size());r.queue().submit([encoder.finish()]);
    let a=read(&r,&retained);
    let (flipped,_)=r.finish_metalfx_texture(128,72,true,true).unwrap();let b=read(&r,&flipped);
    assert_eq!(r.timings.guides_ms,timings.guides_ms,"layer masking reuses guides");
    assert_eq!(r.timings.metalfx_ms,timings.metalfx_ms,"layer masking reuses MetalFX");
    assert!(r.timings.post_ms>timings.post_ms);
    assert!(a.chunks_exact(4).any(|p|p[0]!=p[1]));
    assert_eq!(a.chunks_exact(128*4).rev().flatten().copied().collect::<Vec<_>>(),b);
    assert_eq!(a,read(&r,&retained),"later layers must not replace the retained image");
    r.trace(1,3,true,6);
    assert_eq!(r.timings.metalfx_ms,0.,"new trace retires previous stage timings");
}
