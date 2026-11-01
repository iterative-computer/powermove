//! A native transport feed. This updates CALayers without notifying GPUI.
use crate::session::Controller;
use gpui::*;
use objc2::{MainThreadMarker, msg_send};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use pm_engine_service::layers::PlaybackLayers;
use pm_shell::layout::Rect;
use std::{cell::RefCell, collections::HashMap, time::Duration};
thread_local! {static TIMELINES: RefCell<HashMap<EntityId,(Rect,f32,f32)>>=RefCell::new(HashMap::new());}
pub fn set_timeline(id: EntityId, r: Rect, x: f32, y: f32) {
    TIMELINES.with(|t| {
        t.borrow_mut().insert(id, (r, x, y));
    });
}

pub fn install(entity: &Entity<Controller>, cx: &mut App) {
    let layers = PlaybackLayers::new(MainThreadMarker::new().unwrap());
    let host = entity.read(cx).host.clone();
    if let Some(parent) = host.borrow().gpui.layer() {
        unsafe {
            let _: () = msg_send![&*parent,addSublayer:&*layers.timecode];
            let _: () = msg_send![&*parent,addSublayer:&*layers.playhead];
            let _: () =
                msg_send![&*layers.timecode,setAlignmentMode:objc2_quartz_core::kCAAlignmentRight];
            let color = objc2_app_kit::NSColor::whiteColor().CGColor();
            let _: () = msg_send![&*layers.timecode,setForegroundColor:&*color];
            let accent = host.borrow().tokens.colors.accent;
            let color = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
                ((accent >> 16) & 255) as f64 / 255.,
                ((accent >> 8) & 255) as f64 / 255.,
                (accent & 255) as f64 / 255.,
                1.,
            )
            .CGColor();
            let _: () = msg_send![&*layers.playhead,setBackgroundColor:&*color];
        }
    }
    let weak = entity.downgrade();
    cx.spawn(async move|cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(16)).await;
            if weak.update(cx, |v,cx| {
                let Some(viewer)=&v.viewer else{return};
                let status=viewer.status();
                if let Some(error)=status.error {
                    let error=format!("Viewer: {error}");
                    if v.message!=error {v.message=error;cx.notify();}
                }
                let scale=v.host.borrow().window.backingScaleFactor();
                let size=v.host.borrow().viewer_layer().drawable_size();
                let output=v.host.borrow().window.screen().and_then(|screen|pm_engine_service::layers::screen_output(MainThreadMarker::new().unwrap(),&screen,size.width.max(1.)as u32,size.height.max(1.)as u32).ok());
                if let Some((output,_))=output {let _=viewer.configure(output);}
                if let Some(clock)=status.clock {
                    let fraction=(clock.media_time.get()as f64/pm_protocol::common::TICKS_PER_SECOND as f64/v.duration).clamp(0.,1.);
                    let _=layers.update(MainThreadMarker::new().unwrap(),&clock,clock.host_time,v.rate,fraction,[0.,0.]);
                    let host=v.host.borrow();
                    let height=host.gpui.bounds().size.height;
                    let width=host.gpui.bounds().size.width;
                    let metrics=host.tokens.metrics;
                    unsafe {
                        let _:()=msg_send![objc2::class!(CATransaction),begin];
                        let _:()=msg_send![objc2::class!(CATransaction),setDisableActions:true];
                        let _:()=msg_send![&*layers.timecode,setContentsScale:scale];
                        let _:()=msg_send![&*layers.timecode,setFontSize:host.tokens.typography.body as f64];
                        let _:()=msg_send![&*layers.timecode,setFrame:NSRect::new(NSPoint::new(width-metrics.min_panel as f64*2.,0.),NSSize::new(metrics.min_panel as f64*2.-metrics.inset as f64,metrics.status as f64))];
                        let geometry=TIMELINES.with(|t|t.borrow().get(&cx.entity_id()).copied());
                        if let Some((r,x,y))=geometry {
                            let _:()=msg_send![&*layers.playhead,setFrame:NSRect::new(NSPoint::new(r.x as f64+x as f64+(r.w-x-metrics.inset)as f64*fraction,height-(r.y+r.h-metrics.inset)as f64),NSSize::new(metrics.border as f64,(r.h-y-metrics.inset).max(0.)as f64))];
                        }
                        let _:()=msg_send![objc2::class!(CATransaction),commit];
                    }
                }
            }).is_err(){break;}
        }
        unsafe {let _:()=msg_send![&*layers.timecode,removeFromSuperlayer];let _:()=msg_send![&*layers.playhead,removeFromSuperlayer];}
    }).detach();
}
