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

/// Playhead head size, points (language §11).
pub const HEAD: f32 = 11.;

pub struct Slot {
    pub w: f32,
    pub h: f32,
}
/// Room the timeline header leaves for the native timecode layer.
pub fn timecode_slot(t: &pm_design::Tokens) -> Slot {
    let size = t.typography.timecode;
    Slot {
        w: (size * 7.5).ceil(),
        h: (size * t.typography.dense_leading).ceil(),
    }
}

/// sRGB token colour as a CGColor (the type lives in objc2-core-graphics).
macro_rules! cg {
    ($color:expr) => {{
        let color: u32 = $color;
        objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
            ((color >> 16) & 255) as f64 / 255.,
            ((color >> 8) & 255) as f64 / 255.,
            (color & 255) as f64 / 255.,
            1.,
        )
        .CGColor()
    }};
}

thread_local! {static OUTLINE: RefCell<Option<objc2::rc::Retained<objc2::runtime::AnyObject>>> = const { RefCell::new(None) };}
/// A `width` pt outline just outside the composition frame (the viewer's
/// Metal layer), shown only when the comp cannot be told from the surround.
pub fn frame_outline(host: &pm_shell::Host, visible: bool, color: u32, width: f32) {
    let metal = host.viewer_layer();
    let parent = foreign_types::ForeignType::as_ptr(&metal) as *mut objc2::runtime::AnyObject;
    OUTLINE.with(|o| {
        let mut o = o.borrow_mut();
        let outline = o.get_or_insert_with(|| unsafe {
            let l: objc2::rc::Retained<objc2::runtime::AnyObject> =
                objc2::rc::Retained::from_raw(msg_send![objc2::class!(CALayer), new]).unwrap();
            let _: () = msg_send![parent, addSublayer: &*l];
            // kCALayerWidthSizable | kCALayerHeightSizable: tracks the shell's frame changes.
            let _: () = msg_send![&*l, setAutoresizingMask: 2u32 | 16u32];
            l
        });
        unsafe {
            let _: () = msg_send![objc2::class!(CATransaction), begin];
            let _: () = msg_send![objc2::class!(CATransaction), setDisableActions: true];
            let bounds: NSRect = msg_send![parent, bounds];
            let w = width as f64;
            let _: () = msg_send![&**outline, setFrame: NSRect::new(
                NSPoint::new(-w, -w),
                NSSize::new(bounds.size.width + w * 2., bounds.size.height + w * 2.))];
            let _: () = msg_send![&**outline, setBorderWidth: w];
            let _: () = msg_send![&**outline, setBorderColor: &*cg!(color)];
            let _: () = msg_send![&**outline, setHidden: !visible];
            let _: () = msg_send![objc2::class!(CATransaction), commit];
        }
    });
}

pub fn install(entity: &Entity<Controller>, cx: &mut App) {
    let layers = PlaybackLayers::new(MainThreadMarker::new().unwrap());
    // The playhead's 11 pt rounded head, drawn in the ruler (language §11).
    let head: objc2::rc::Retained<objc2::runtime::AnyObject> =
        unsafe { objc2::rc::Retained::from_raw(msg_send![objc2::class!(CALayer), new]).unwrap() };
    let host = entity.read(cx).host.clone();
    if let Some(parent) = host.borrow().gpui.layer() {
        unsafe {
            let _: () = msg_send![&*parent,addSublayer:&*layers.timecode];
            let _: () = msg_send![&*parent,addSublayer:&*layers.playhead];
            let _: () =
                msg_send![&*layers.timecode,setAlignmentMode:objc2_quartz_core::kCAAlignmentLeft];
            let t = host.borrow().tokens.clone();
            let font = objc2_app_kit::NSFont::monospacedDigitSystemFontOfSize_weight(
                t.typography.timecode as f64,
                objc2_app_kit::NSFontWeightMedium,
            );
            // CATextLayer.font is a CFTypeRef (`^v`); NSFont is toll-free bridged to CTFont.
            let font_ref = objc2::rc::Retained::as_ptr(&font) as *const std::ffi::c_void;
            let _: () = msg_send![&*layers.timecode,setFont:font_ref];
            let _: () = msg_send![&*layers.timecode,setHidden:true];
            let _: () = msg_send![&*layers.playhead,addSublayer:&*head];
            let radius = t.metrics.inner_radius as f64 * 0.75;
            let _: () = msg_send![&*head,setCornerRadius:radius];
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
    let main_output = entity.read(cx).main_output.clone();
    let binding = pm_engine_service::layers::OutputBinding::new(
        &host.borrow().window,
        host.borrow().viewer_layer(),
        Rc::new(move |output| {
            main_output.set(Some(output));
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
                if let Some(clock)=status.clock { v.sync_popout(&clock); }
                if let Some(clock)=status.clock {
                    let fraction=(clock.media_time.get()as f64/pm_protocol::common::TICKS_PER_SECOND as f64/v.duration).clamp(0.,1.);
                    let _=layers.update(MainThreadMarker::new().unwrap(),&clock,clock.host_time,v.rate,fraction,std::array::from_fn(|i|status.levels.as_ref().and_then(|l|l.master.peak.get(i)).copied().unwrap_or(0.) as f64));
                    let host=v.host.borrow();
                    let height=host.gpui.bounds().size.height;
                    let metrics=host.tokens.metrics;
                    unsafe {
                        let _:()=msg_send![objc2::class!(CATransaction),begin];
                        let _:()=msg_send![objc2::class!(CATransaction),setDisableActions:true];
                        let colors=&host.tokens.colors;
                        let _:()=msg_send![&*layers.timecode,setContentsScale:scale];
                        let _:()=msg_send![&*layers.timecode,setFontSize:host.tokens.typography.timecode as f64];
                        let _:()=msg_send![&*layers.timecode,setForegroundColor:&*cg!(colors.text)];
                        let _:()=msg_send![&*layers.playhead,setBackgroundColor:&*cg!(colors.accent)];
                        let _:()=msg_send![&*head,setBackgroundColor:&*cg!(colors.accent)];
                        let _:()=msg_send![&*head,setContentsScale:scale];
                        let geometry=TIMELINES.with(|t|t.borrow().get(&cx.entity_id()).copied());
                        let _:()=msg_send![&*layers.timecode,setHidden:geometry.is_none()];
                        let _:()=msg_send![&*layers.playhead,setHidden:geometry.is_none()];
                        if let Some((r,x,y))=geometry {
                            // Timecode sits in the timeline header (the shell's) after the play button.
                            let slot=timecode_slot(&host.tokens);
                            let top=r.y+(metrics.header-slot.h)/2.;
                            let _:()=msg_send![&*layers.timecode,setFrame:NSRect::new(NSPoint::new((r.x+metrics.inset+metrics.hit+metrics.gap) as f64,height-(top+slot.h) as f64),NSSize::new(slot.w as f64,slot.h as f64))];
                            let line=metrics.playhead as f64;
                            let tall=(r.h-y-metrics.inset).max(0.) as f64;
                            let at=r.x as f64+x as f64+(r.w-x-metrics.inset)as f64*fraction-line/2.;
                            let _:()=msg_send![&*layers.playhead,setFrame:NSRect::new(NSPoint::new(at,height-(r.y+r.h-metrics.inset)as f64),NSSize::new(line,tall))];
                            let size=HEAD as f64;
                            let _:()=msg_send![&*head,setFrame:NSRect::new(NSPoint::new((line-size)/2.,tall-size),NSSize::new(size,size))];
                        }
                        let _:()=msg_send![objc2::class!(CATransaction),commit];
                    }
                }
            }).is_err(){break;}
        }
        unsafe {let _:()=msg_send![&*layers.timecode,removeFromSuperlayer];let _:()=msg_send![&*layers.playhead,removeFromSuperlayer];let _:()=msg_send![&*head,removeFromSuperlayer];}
    }).detach();
}
