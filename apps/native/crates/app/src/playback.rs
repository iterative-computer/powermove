//! A native transport feed. This updates CALayers without notifying GPUI.
use crate::session::Controller;
use gpui::*;
use objc2::{MainThreadMarker, msg_send};
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use pm_engine_service::layers::PlaybackLayers;
use std::{cell::RefCell, rc::Rc};
/// When panels hear about the transport: every paused time change, and only
/// the start and end of playback (S3: no GPUI repaint while playing).
#[derive(Default)]
pub struct PanelClock {
    time: f64,
    playing: bool,
}
impl PanelClock {
    pub fn due(&mut self, time: f64, playing: bool) -> bool {
        let due = playing != self.playing || (!playing && time != self.time);
        self.time = time;
        self.playing = playing;
        due
    }
}

pub struct Slot {
    pub w: f32,
    pub h: f32,
}
/// The native timecode layer's box: exactly the width of `00:00:00:00` in the
/// timecode font (tabular digits, so every time fits), so its opaque ground
/// covers the paused readout and never the frame rate beside it.
pub fn timecode_slot(t: &pm_design::Tokens) -> Slot {
    let font = objc2_app_kit::NSFont::monospacedDigitSystemFontOfSize_weight(
        t.typography.timecode as f64,
        unsafe { objc2_app_kit::NSFontWeightMedium },
    );
    let sample = objc2_foundation::NSString::from_str("00:00:00:00");
    let key: &objc2_foundation::NSString = unsafe { objc2_app_kit::NSFontAttributeName };
    let attributes = objc2_foundation::NSDictionary::<
        objc2_foundation::NSString,
        objc2::runtime::AnyObject,
    >::from_slices(&[key], &[&*font as &objc2::runtime::AnyObject]);
    let size: NSSize = unsafe { msg_send![&*sample, sizeWithAttributes: &*attributes] };
    Slot {
        w: size.width.ceil() as f32 + t.metrics.border,
        h: (t.typography.timecode * t.typography.dense_leading).ceil(),
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

/// A `width` pt outline just outside the composition frame (the viewer's
/// Metal layer), shown only when the comp cannot be told from the surround.
#[derive(Default)]
pub struct FrameOutline(Option<Retained<AnyObject>>);
impl FrameOutline {
    pub fn show(&mut self, host: &pm_shell::Host, visible: bool, color: u32, width: f32) {
        let metal = host.viewer_layer();
        let parent = foreign_types::ForeignType::as_ptr(&metal) as *mut AnyObject;
        let outline = self.0.get_or_insert_with(|| unsafe {
            let l: Retained<AnyObject> =
                Retained::from_raw(msg_send![objc2::class!(CALayer), new]).unwrap();
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
    }
}
impl Drop for FrameOutline {
    fn drop(&mut self) {
        if let Some(layer) = &self.0 {
            unsafe {
                let _: () = msg_send![&**layer, removeFromSuperlayer];
            }
        }
    }
}

pub fn install(entity: &Entity<Controller>, cx: &mut App) {
    let layers = PlaybackLayers::new(MainThreadMarker::new().unwrap());
    let host = entity.read(cx).host.clone();
    let mut playhead = None;
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
            playhead = Some(crate::playhead::Playhead::new(
                &parent,
                layers.playhead.clone(),
            ));
        }
    }
    let (tx, rx) = async_channel::bounded(1);
    // Home placing or lifting its cover moves the playhead without a tick.
    let wake = tx.clone();
    entity.update(cx, |v, _| v.playback_wake = Some(wake));
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
    let crop = host.borrow().viewer_crop.clone();
    let publish_output: Rc<dyn Fn(pm_engine_service::Output)> = Rc::new(move |output| {
        // Zoomed past its pane, the drawable is a window onto the canvas.
        let output = pm_engine_service::Output {
            crop: crop.get().map(
                |[x, y, canvas_width, canvas_height]| pm_engine_service::Crop {
                    x,
                    y,
                    canvas_width,
                    canvas_height,
                },
            ),
            ..output
        };
        main_output.set(Some(output));
        if let Some(v) = &*placed.borrow() {
            let _ = v.configure(output);
        }
    });
    // A project owns this binding and its layer. Home must not keep the
    // old layer's drawable pool alive through the window-lifetime feed.
    let mut binding = None::<pm_engine_service::layers::OutputBinding>;
    let weak = entity.downgrade();
    // The viewer's last status line shown (an error, or what a frame leaves
    // out): set when it changes, cleared only while no other message
    // replaced it.
    let mut shown = None::<String>;
    let mut studio_status = None;
    let mut measured = None::<(f32, Slot)>;
    cx.spawn(async move|cx| {
        loop {
            if rx.recv().await.is_err() { break; }
            if weak.update(cx, |v,cx| {
                let Some(viewer)=&v.viewer else{
                    link.set_paused(true); *current.borrow_mut()=None; binding=None;
                    // A closed project (Home stays in the window) leaves no playhead.
                    unsafe {let _:()=msg_send![&*layers.timecode,setHidden:true];let _:()=msg_send![&*layers.playhead,setHidden:true];}
                    return
                };
                let new_viewer = current.borrow().as_ref().map(|v|v.identity()) != Some(viewer.identity());
                if new_viewer {
                    *current.borrow_mut() = Some(viewer.clone());
                    binding = Some(pm_engine_service::layers::OutputBinding::new(
                        &host.borrow().window,
                        host.borrow().viewer_layer(),
                        publish_output.clone(),
                    ));
                    binding.as_ref().unwrap().sync();
                    if std::env::var_os("PM_ENGINE_TRACE").is_some() {
                        let _ = viewer.command(pm_protocol::clock::TransportCommand::SetLoop { range: Some(pm_protocol::common::TickRange {
                            start: pm_protocol::common::Ticks::ZERO,
                            end: pm_protocol::common::Ticks::new((v.duration * pm_protocol::common::TICKS_PER_SECOND as f64) as i64).unwrap_or(pm_protocol::common::Ticks::from_whole_seconds(1))
                        }) });
                    }
                }
                if let Some((target, interval)) = ticks.borrow_mut().take() { let _=viewer.display_tick(target, interval); }
                let status=viewer.status();
                let next_studio_status=(status.refine_progress,status.export_estimate.clone());
                if studio_status.as_ref()!=Some(&next_studio_status) {
                    studio_status=Some(next_studio_status);
                    v.update_builtins(cx);
                }
                v.tell_presented(status.frame.as_ref());
                // Run through the audio-start wait too: its vsync phase places the start.
                link.set_paused(!status.wants_display);
                // One readable line; a longer report goes to Window ▸ Error Log.
                let line=status.error.as_ref().map(|e|{
                    let (first,rest)=e.split_once('\n').unwrap_or((e,""));
                    if rest.trim().is_empty() { format!("Viewer: {first}") } else {
                        crate::error_log::shared(cx).borrow_mut().push("Viewer",e);
                        format!("Viewer: {first} Details are in Window ▸ Error Log.")
                    }
                }).or_else(||failures_line(&status, v.published.as_deref())).or_else(||status.notice());
                if line!=shown {
                    // Layers newly left out (or back): Properties marks them.
                    v.update_builtins(cx);
                    match &line {
                        Some(text)=>{v.message=text.clone();cx.notify();}
                        None if shown.as_ref()==Some(&v.message)=>{v.message.clear();cx.notify();}
                        None=>{}
                    }
                    shown=line;
                }
                v.tick_builtins(cx);
                let scale=v.host.borrow().window.backingScaleFactor();
                if let Some(clock)=status.clock { v.watch_run(&clock); v.sync_popout(&clock); v.sync_studio_pane_clock(&clock); }
                v.sync_studio_panes(cx);
                if let Some(clock)=status.clock {
                    // Shuttling is a run too: the live timecode shows at every speed.
                    let playing=clock.state!=pm_protocol::clock::TransportState::Stopped;
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
                        if let Some(p)=&playhead { p.style(&host.tokens, scale); }
                        let geometry=v.builtins.timeline_place;
                        let (pps,scroll_time,column)=v.builtins.timeline.as_ref().map_or((100.,0.,0.),|t|t.read(cx).time_mapping());
                        // The playhead ends where the tracks do, above the bottom bar.
                        let bar=v.builtins.timeline.as_ref().map_or(0.,|t|t.read(cx).bar_height()) as f32;
                        let seconds=clock.media_time.get()as f64/pm_protocol::common::TICKS_PER_SECOND as f64;
                        let offset=column+(seconds-scroll_time)*pps;
                        // A clear ground: the panel hides its paused readout while playing
                        // (it hears only play start and stop), so the translucent header
                        // shows through unchanged.
                        let _:()=msg_send![&*layers.timecode,setBackgroundColor:&*crate::playhead::colour(colors.panel, 0.)];
                        let _:()=msg_send![&*layers.timecode,setHidden:geometry.is_none()||!playing];
                        let _:()=msg_send![&*layers.playhead,setHidden:geometry.is_none_or(|(r,_)|offset<column||offset>r.w as f64)];
                        if let Some((r,y))=geometry {
                            // Measured once per timecode size, never per tick.
                            if measured.as_ref().is_none_or(|(size,_)|*size!=host.tokens.typography.timecode) {
                                measured=Some((host.tokens.typography.timecode,timecode_slot(&host.tokens)));
                            }
                            let slot=&measured.as_ref().unwrap().1;
                            let top=r.y+(metrics.header-slot.h)/2.;
                            let _:()=msg_send![&*layers.timecode,setFrame:NSRect::new(NSPoint::new((r.x+metrics.inset+metrics.hit+metrics.gap) as f64,height-(top+slot.h) as f64),NSSize::new(slot.w as f64,slot.h as f64))];
                            let foot=bar.max(metrics.inset);
                            let tall=(r.h-y-foot).max(0.) as f64;
                            if let Some(p)=&mut playhead {
                                p.place(r.x as f64+offset,height-(r.y+r.h-foot)as f64,metrics.playhead as f64,tall,metrics.ruler as f64,playing,&host.tokens);
                            }
                        }
                        let _:()=msg_send![objc2::class!(CATransaction),commit];
                    }
                }
            }).is_err(){break;}
        }
        drop(playhead);
        unsafe {let _:()=msg_send![&*layers.timecode,removeFromSuperlayer];let _:()=msg_send![&*layers.playhead,removeFromSuperlayer];}
    }).detach();
}

/// The status line for layers a frame leaves out, by name, pointing at
/// where the reason is (Properties shows each layer's own, in words).
fn failures_line(
    status: &pm_engine_service::ViewerStatus,
    published: Option<&pm_kernel::Published>,
) -> Option<String> {
    let mut ids: Vec<&str> = vec![];
    for f in &status.layer_failures {
        if !ids.contains(&f.layer.as_str()) {
            ids.push(&f.layer);
        }
    }
    let first = *ids.first()?;
    let name = published
        .and_then(|p| {
            let root = p.snapshot.root()?;
            let at = pm_kernel::model::find_layer(root, first)?;
            let layer = pm_kernel::model::layer_at(root, &at)?.to_value();
            layer
                .get("name")
                .and_then(pm_model::Value::as_str)
                .map(str::to_owned)
        })
        .filter(|n| !n.is_empty());
    Some(match (ids.len(), name) {
        (1, Some(name)) => format!("“{name}” isn’t drawn. Select it to see why in Properties."),
        (1, None) => "1 layer isn’t drawn. Select it to see why in Properties.".into(),
        (n, Some(name)) => format!("“{name}” and {} more aren’t drawn. Select one to see why in Properties.", n - 1),
        (n, None) => format!("{n} layers aren’t drawn. Select one to see why in Properties."),
    })
}
