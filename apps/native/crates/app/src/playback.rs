//! A native transport feed. This updates CALayers without notifying GPUI.
use crate::session::Controller;
use gpui::*;
use objc2::{MainThreadMarker, msg_send};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use pm_engine_service::layers::PlaybackLayers;
use pm_shell::layout::Rect;
use std::{cell::RefCell, collections::HashMap, rc::Rc};
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
    let (tx, rx) = async_channel::bounded(1);
    let ticks = Rc::new(RefCell::new(None));
    let pending = ticks.clone();
    let tick_tx = tx.clone();
    let hz = std::env::var("PM_NATIVE_VERIFY_HZ")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120.);
    let link = pm_engine_service::layers::DisplayLink::new(
        &host.borrow().gpui,
        hz,
        Rc::new(move |target, interval| {
            *pending.borrow_mut() = Some((target, interval));
            let _ = tick_tx.try_send(());
        }),
    );
    let notifications = entity.read(cx).engine.notifications();
    cx.spawn(async move |_| {
        while notifications.recv().await.is_ok() {
            if tx.send(()).await.is_err() {
                break;
            }
        }
    })
    .detach();
    let current = Rc::new(RefCell::new(None::<pm_engine_service::Viewer>));
    let placed = current.clone();
    let binding = pm_engine_service::layers::OutputBinding::new(
        &host.borrow().window,
        host.borrow().viewer_layer(),
        Rc::new(move |output| {
            if let Some(v) = &*placed.borrow() {
                let _ = v.configure(output);
            }
        }),
    );
    let weak = entity.downgrade();
    cx.spawn(async move|cx| {
        loop {
            if rx.recv().await.is_err() { break; }
            if weak.update(cx, |v,cx| {
                let Some(viewer)=&v.viewer else{link.set_paused(true); *current.borrow_mut()=None; return};
                let new_viewer = current.borrow().as_ref().map(|v|v.identity()) != Some(viewer.identity());
                if new_viewer {
                    *current.borrow_mut() = Some(viewer.clone()); binding.sync();
                    if std::env::var_os("PM_ENGINE_TRACE").is_some() {
                        let _ = viewer.command(pm_protocol::clock::TransportCommand::SetLoop { range: Some(pm_protocol::common::TickRange {
                            start: pm_protocol::common::Ticks::ZERO,
                            end: pm_protocol::common::Ticks::new((v.duration * pm_protocol::common::TICKS_PER_SECOND as f64) as i64).unwrap_or(pm_protocol::common::Ticks::from_whole_seconds(1))
                        }) });
                    }
                }
                if let Some((target, interval)) = ticks.borrow_mut().take() { let _=viewer.display_tick(target, interval); }
                let status=viewer.status();
                // Run through the audio-start wait too: its vsync phase places the start.
                link.set_paused(!status.wants_display);
                if let Some(error)=status.error {
                    let error=format!("Viewer: {error}");
                    if v.message!=error {v.message=error;cx.notify();}
                }
                let scale=v.host.borrow().window.backingScaleFactor();
                if let Some(clock)=status.clock {
                    let fraction=(clock.media_time.get()as f64/pm_protocol::common::TICKS_PER_SECOND as f64/v.duration).clamp(0.,1.);
                    let _=layers.update(MainThreadMarker::new().unwrap(),&clock,clock.host_time,v.rate,fraction,std::array::from_fn(|i|status.levels.as_ref().and_then(|l|l.master.peak.get(i)).copied().unwrap_or(0.) as f64));
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
