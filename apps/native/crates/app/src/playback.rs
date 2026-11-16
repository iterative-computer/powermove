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

/// The clock the head and timecode show: paused during a ruler scrub, the
/// time the scrub last asked for (`asked`, seconds), not yet the viewer's.
pub fn scrub_shown(
    mut clock: pm_protocol::clock::ClockSample,
    playing: bool,
    asked: Option<f64>,
) -> pm_protocol::clock::ClockSample {
    if !playing
        && let Some(seconds) = asked.filter(|s| s.is_finite() && *s >= 0.)
        && let Ok(ticks) = pm_protocol::common::Ticks::new(
            (seconds * pm_protocol::common::TICKS_PER_SECOND as f64).round() as i64,
        )
    {
        clock.media_time = ticks;
    }
    clock
}

/// The host instant a playing playhead and timecode stand for: the next
/// vsync (`vsync`, CADisplayLink host seconds, the same clock as
/// `HostMicros`), never before the sample's own `host_time`. Paused, the
/// sample's time.
pub fn shown_at(
    clock: &pm_protocol::clock::ClockSample,
    vsync: Option<f64>,
    playing: bool,
) -> pm_protocol::common::HostMicros {
    match vsync {
        Some(target) if playing && target.is_finite() && target > 0. => {
            pm_protocol::common::HostMicros::new((target * 1e6) as u64)
                .map_or(clock.host_time, |t| t.max(clock.host_time))
        }
        _ => clock.host_time,
    }
}

pub struct Slot {
    pub w: f32,
    pub h: f32,
}
/// The native timecode layer's box: exactly the width of `00:00:00:00` in the
/// timecode font (tabular digits, so every time fits), the slot the timeline's
/// readout keeps for it, never over the frame rate beside it.
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

/// The picture a project's viewer shows from when its viewer is bound until
/// its first real frame (PM-384): the frame the viewer last showed for it
/// (`crate::poster::last_frame_files`), drawn over the viewer's layer, as
/// the viewer fits its picture. Gone with the first frame of the project.
struct Placeholder {
    layer: Retained<AnyObject>,
    doc_id: u64,
}
impl Placeholder {
    fn over(viewer: &metal::MetalLayer, poster: &std::path::Path, doc_id: u64) -> Option<Self> {
        let path = objc2_foundation::NSString::from_str(&poster.to_string_lossy());
        let image = objc2_app_kit::NSImage::initWithContentsOfFile(
            <objc2_app_kit::NSImage as objc2::AnyThread>::alloc(),
            &path,
        )?;
        unsafe {
            let layer: Retained<AnyObject> =
                Retained::from_raw(msg_send![objc2::class!(CALayer), new])?;
            let parent = foreign_types::ForeignType::as_ptr(viewer) as *mut AnyObject;
            let bounds: NSRect = msg_send![parent, bounds];
            let _: () = msg_send![objc2::class!(CATransaction), begin];
            let _: () = msg_send![objc2::class!(CATransaction), setDisableActions: true];
            // A layer on macOS takes an NSImage as its contents.
            let _: () = msg_send![&*layer, setContents: &*image];
            let _: () = msg_send![&*layer, setContentsGravity: objc2_quartz_core::kCAGravityResizeAspect];
            let _: () = msg_send![&*layer, setFrame: bounds];
            // Width and height follow the viewer (kCALayerWidthSizable | kCALayerHeightSizable).
            let _: () = msg_send![&*layer, setAutoresizingMask: 0x2u32 | 0x10u32];
            let _: () = msg_send![parent, addSublayer: &*layer];
            let _: () = msg_send![objc2::class!(CATransaction), commit];
            Some(Self { layer, doc_id })
        }
    }
}
/// Whether a viewer bound again keeps the placeholder it has: one for the
/// same document, when no new poster was staged for it.
fn placeholder_kept(shown: Option<u64>, staged: bool, doc: Option<u64>) -> bool {
    !staged && shown.is_some() && shown == doc
}
impl Placeholder {
    fn show(&self, on: bool) {
        unsafe {
            let hidden: bool = msg_send![&*self.layer, isHidden];
            if hidden == on {
                let _: () = msg_send![objc2::class!(CATransaction), begin];
                let _: () = msg_send![objc2::class!(CATransaction), setDisableActions: true];
                let _: () = msg_send![&*self.layer, setHidden: !on];
                let _: () = msg_send![objc2::class!(CATransaction), commit];
            }
        }
    }
}
impl Drop for Placeholder {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.layer, removeFromSuperlayer];
        }
    }
}

pub fn install(entity: &Entity<Controller>, cx: &mut App) {
    let layers = PlaybackLayers::new(MainThreadMarker::new().unwrap());
    // The viewer bar's time while playing: the timeline's timecode, again.
    let viewer_time: Retained<AnyObject> =
        unsafe { Retained::from_raw(msg_send![objc2::class!(CATextLayer), new]).unwrap() };
    let host = entity.read(cx).host.clone();
    let mut playhead = None;
    let mut skimmer = None;
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
            let _: () = msg_send![&*parent,addSublayer:&*viewer_time];
            let _: () =
                msg_send![&*viewer_time,setAlignmentMode:objc2_quartz_core::kCAAlignmentLeft];
            let _: () = msg_send![&*viewer_time,setFont:font_ref];
            let _: () = msg_send![&*viewer_time,setHidden:true];
            skimmer = Some(crate::playhead::Skimmer::new(&parent, &t));
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
    // The display link's next vsync (host seconds): where a playing playhead
    // and timecode will be seen, rather than where the last sample stood.
    let mut vsync = None::<f64>;
    let mut placeholder = None::<Placeholder>;
    cx.spawn(async move|cx| {
        loop {
            if rx.recv().await.is_err() { break; }
            if weak.update(cx, |v,cx| {
                let Some(viewer)=&v.viewer else{
                    link.set_paused(true); *current.borrow_mut()=None; binding=None; placeholder=None;
                    // A closed project (Home stays in the window) leaves no playhead.
                    unsafe {let _:()=msg_send![&*layers.timecode,setHidden:true];let _:()=msg_send![&*layers.playhead,setHidden:true];let _:()=msg_send![&*viewer_time,setHidden:true];}
                    if let Some(s)=&mut skimmer { s.hide(); }
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
                    // The opening project's last picture stands in until its
                    // first frame, also across a viewer bound again for the
                    // same document before that frame (PM-384).
                    let doc = v.published.as_ref().map(|p| p.snapshot.doc_id);
                    let poster = v.viewer_poster.take();
                    // The layer it sits on is the window's, kept across viewers.
                    if !placeholder_kept(placeholder.as_ref().map(|p| p.doc_id), poster.is_some(), doc) {
                        placeholder = poster.zip(doc)
                            .and_then(|(poster, doc)| Placeholder::over(&host.borrow().viewer_layer(), &poster, doc));
                    }
                    crate::diag::line(serde_json::json!({"placeholder": placeholder.is_some()}));
                    if std::env::var_os("PM_ENGINE_TRACE").is_some() {
                        let _ = viewer.command(pm_protocol::clock::TransportCommand::SetLoop { range: Some(pm_protocol::common::TickRange {
                            start: pm_protocol::common::Ticks::ZERO,
                            end: pm_protocol::common::Ticks::new((v.duration() * pm_protocol::common::TICKS_PER_SECOND as f64) as i64).unwrap_or(pm_protocol::common::Ticks::from_whole_seconds(1))
                        }) });
                    }
                }
                if let Some((target, interval)) = ticks.borrow_mut().take() { vsync=Some(target); let _=viewer.display_tick(target, interval); }
                let status=viewer.status();
                // Shown only once the viewer's ghost is gone: while the panel
                // shows its ghost (with the same picture, at its own size),
                // both would show, or neither when the ghost's went early.
                if let Some(p) = &placeholder {
                    p.show(!crate::panels::ghost_shows(v, "viewer"));
                }
                let next_studio_status=(status.refine_progress,status.export_estimate.clone());
                if studio_status.as_ref()!=Some(&next_studio_status) {
                    studio_status=Some(next_studio_status);
                    v.update_builtins(cx);
                }
                v.tell_presented(status.frame.as_ref());
                v.watch_still(status.frame.as_ref(), cx);
                // The front window's restored project is on screen: the
                // other restored windows open theirs now.
                if status.frame.as_ref().is_some_and(|f| v.published.as_ref().is_some_and(|p| p.snapshot.doc_id == f.stamp.doc_id)) {
                    v.release_front();
                }
                if placeholder.as_ref().is_some_and(|p| status.frame.as_ref().is_some_and(|f| f.stamp.doc_id == p.doc_id)) {
                    placeholder = None;
                    crate::diag::line(serde_json::json!({"placeholder": "first frame"}));
                }
                if !status.layer_failures.is_empty() {
                    let contributions = v.extensions.host.as_ref().and_then(|host| host.contributions());
                    record_layer_failures(&mut v.errors.borrow_mut(), &status.layer_failures, |failure| {
                        layer_failure_source(failure, v.published.as_deref(), contributions.as_deref())
                    });
                }
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
                    // Playing, the head and timecode show the time at the vsync this
                    // tick lands on. A sample's own time only moves when a new one
                    // arrives, so a late sample held the head a frame, then jumped two.
                    // Never before the sample itself (a run's pre-roll is ahead).
                    // Scrubbing the ruler, the head and timecode stand where the
                    // pointer asked, each refresh: the viewer's clock lands with
                    // its picture, which may come later (PM-310).
                    let clock=scrub_shown(clock,playing,v.builtins.scrubbing.then_some(v.builtins.scrub_time).flatten());
                    let at=shown_at(&clock,vsync,playing);
                    let head_time=clock.media_time_at(at);
                    let fraction=(head_time.get()as f64/pm_protocol::common::TICKS_PER_SECOND as f64/v.duration()).clamp(0.,1.);
                    // One transaction for the engine's layer feed and this placement.
                    // The feed sets the playhead layer to a 1×1 progress frame; committed
                    // apart, a vsync between the two showed the head as that dot (gone).
                    unsafe {
                        let _:()=msg_send![objc2::class!(CATransaction),begin];
                        let _:()=msg_send![objc2::class!(CATransaction),setDisableActions:true];
                    }
                    let _=layers.update(MainThreadMarker::new().unwrap(),&clock,at,v.rate,fraction,std::array::from_fn(|i|status.levels.as_ref().and_then(|l|l.master.peak.get(i)).copied().unwrap_or(0.) as f64));
                    let host=v.host.borrow();
                    let height=host.gpui.bounds().size.height;
                    let metrics=host.tokens.metrics;
                    unsafe {
                        let colors=&host.tokens.colors;
                        let _:()=msg_send![&*layers.timecode,setContentsScale:scale];
                        let _:()=msg_send![&*layers.timecode,setFontSize:host.tokens.typography.timecode as f64];
                        // The text ink, as the paused readout (the accent marks the playhead alone).
                        let _:()=msg_send![&*layers.timecode,setForegroundColor:&*cg!(colors.text)];
                        if let Some(p)=&playhead { p.style(&host.tokens, scale); }
                        let geometry=v.builtins.timeline_place;
                        let seconds=head_time.get()as f64/pm_protocol::common::TICKS_PER_SECOND as f64;
                        // Playing, the timeline pages to keep its head in sight (v1).
                        if playing && let Some(t)=&v.builtins.timeline { t.update(cx,|t,cx|t.follow_playhead(seconds,cx)); }
                        let (pps,scroll_time,column)=v.builtins.timeline.as_ref().map_or((100.,0.,0.),|t|t.read(cx).time_mapping());
                        // The playhead ends where the tracks do, above the bottom bar.
                        let bar=v.builtins.timeline.as_ref().map_or(0.,|t|t.read(cx).bar_height()) as f32;
                        let offset=column+(seconds-scroll_time)*pps;
                        // A clear ground over the translucent header. The time is this
                        // layer's paused too: the panel hears play start and stop only
                        // later, so trading the readout back and forth showed both times
                        // at once, or neither. Only the open time field takes its place.
                        let _:()=msg_send![&*layers.timecode,setBackgroundColor:&*crate::playhead::colour(colors.panel, 0.)];
                        let typing=v.builtins.views.get("timeline").is_some_and(|t|t.read(cx).field_open("timeline:time"));
                        let _:()=msg_send![&*layers.timecode,setHidden:geometry.is_none()||typing];
                        let viewer_bar=v.builtins.viewer_place;
                        let text:*mut AnyObject=msg_send![&*layers.timecode,string];
                        let _:()=msg_send![&*viewer_time,setString:text];
                        let _:()=msg_send![&*viewer_time,setContentsScale:scale];
                        let _:()=msg_send![&*viewer_time,setFontSize:host.tokens.typography.timecode as f64];
                        let _:()=msg_send![&*viewer_time,setForegroundColor:&*cg!(colors.text)];
                        // With a timeline on screen its head shows the time: the viewer shows none.
                        let _:()=msg_send![&*viewer_time,setHidden:viewer_bar.is_none()||!playing||geometry.is_some()||v.timeline_shown==Some(true)];
                        let head_shown=geometry.is_some_and(|(r,_)|offset>=column&&offset<=r.w as f64);
                        let _:()=msg_send![&*layers.playhead,setHidden:!head_shown];
                        // Measured once per timecode size, never per tick.
                        if measured.as_ref().is_none_or(|(size,_)|*size!=host.tokens.typography.timecode) {
                            measured=Some((host.tokens.typography.timecode,timecode_slot(&host.tokens)));
                        }
                        let slot=&measured.as_ref().unwrap().1;
                        if let Some(b)=viewer_bar {
                            let top=b.y+(metrics.header-slot.h)/2.;
                            let _:()=msg_send![&*viewer_time,setFrame:NSRect::new(NSPoint::new((b.x+metrics.inset) as f64,height-(top+slot.h) as f64),NSSize::new(slot.w as f64,slot.h as f64))];
                        }
                        if geometry.is_none() && let Some(p)=&mut playhead { p.hide(); }
                        if let Some((r,y))=geometry {
                            // The timeline's head sits over the layer column, beside the
                            // ruler and under the tab row (`y`): play, then the time.
                            let top=r.y+y+(metrics.ruler-slot.h)/2.;
                            let _:()=msg_send![&*layers.timecode,setFrame:NSRect::new(NSPoint::new((r.x+metrics.inset+metrics.hit+metrics.gap) as f64,height-(top+slot.h) as f64),NSSize::new(slot.w as f64,slot.h as f64))];
                            let foot=bar.max(metrics.inset);
                            let tall=(r.h-y-foot).max(0.) as f64;
                            if let Some(p)=&mut playhead {
                                if head_shown {
                                    p.place(r.x as f64+offset,seconds*pps,r.x as f64+column,height-(r.y+r.h-foot)as f64,metrics.playhead as f64,tall,metrics.ruler as f64,playing,&host.tokens);
                                    // The Value Graph's readouts ride on the head (v1).
                                    let marks=v.builtins.timeline.as_ref().map(|t|t.read(cx).graph_marks(seconds)).unwrap_or_default();
                                    p.marks(&marks,height,scale,&host.tokens);
                                }
                                else { p.hide(); }
                            }
                        }
                        // The skimmer, as the head: outside GPUI, so a scroll under a
                        // resting pointer moves the time under it with no panel draw.
                        let skim=v.builtins.timeline.as_ref().and_then(|t|t.read(cx).skimmer_mark());
                        if let Some(s)=&mut skimmer {
                            match (geometry,skim) {
                                (Some((r,y)),Some((x,code)))=>s.place(r.x as f64+x,(r.y+y) as f64,(r.y+r.h) as f64,&code,height,scale,&host.tokens),
                                _=>s.hide(),
                            }
                        }
                        let _:()=msg_send![objc2::class!(CATransaction),commit];
                    }
                } else if v.builtins.timeline_place.is_none() {
                    // No clock yet (or none any more): still nothing of a
                    // timeline that is not on screen.
                    unsafe {let _:()=msg_send![&*layers.timecode,setHidden:true];}
                    if let Some(p)=&mut playhead { p.hide(); }
                    if let Some(s)=&mut skimmer { s.hide(); }
                }
            }).is_err(){break;}
        }
        drop(playhead);
        drop(skimmer);
        unsafe {let _:()=msg_send![&*layers.timecode,removeFromSuperlayer];let _:()=msg_send![&*layers.playhead,removeFromSuperlayer];let _:()=msg_send![&*viewer_time,removeFromSuperlayer];}
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
        (n, Some(name)) => format!(
            "“{name}” and {} more aren’t drawn. Select one to see why in Properties.",
            n - 1
        ),
        (n, None) => format!("{n} layers aren’t drawn. Select one to see why in Properties."),
    })
}

#[cfg(test)]
mod shown_at_tests {
    use super::{scrub_shown, shown_at};
    use pm_protocol::clock::{ClockSample, TransportState};
    use pm_protocol::common::{HostMicros, TICKS_PER_SECOND};

    fn sample(state: TransportState, host_us: u64, media_s: i64) -> ClockSample {
        ClockSample {
            seq: pm_protocol::common::Seq::new(1).unwrap(),
            epoch: 1,
            state,
            host_time: HostMicros::new(host_us).unwrap(),
            media_time: pm_protocol::common::Ticks::new(media_s * TICKS_PER_SECOND).unwrap(),
            rate: pm_protocol::common::PlaybackRate::NORMAL,
            output_latency_us: 0,
            sample_rate: 48_000,
            loop_range: None,
        }
    }

    #[test]
    fn a_playing_head_shows_the_time_at_the_next_vsync() {
        // A sample from 1.000 s, drawn for the vsync at 1.050 s: 50 ms on.
        let c = sample(TransportState::Playing, 1_000_000, 10);
        let at = shown_at(&c, Some(1.05), true);
        assert_eq!(at.get(), 1_050_000);
        let media = c.media_time_at(at).get();
        assert_eq!(media, 10 * TICKS_PER_SECOND + TICKS_PER_SECOND / 20);
    }

    #[test]
    fn never_before_the_sample_and_paused_stays_put() {
        // Pre-roll: the sample is anchored ahead of the vsync. Use its own time.
        let c = sample(TransportState::Playing, 2_000_000, 3);
        assert_eq!(shown_at(&c, Some(1.9), true), c.host_time);
        let paused = sample(TransportState::Stopped, 1_000_000, 3);
        assert_eq!(shown_at(&paused, Some(5.0), false), paused.host_time);
        assert_eq!(shown_at(&c, None, true), c.host_time);
    }

    /// PM-310: scrubbing, the head shows the time the pointer asked for
    /// while the viewer's clock still has the last picture's; playing, or
    /// with no scrub, the clock's own.
    #[test]
    fn a_scrub_shows_the_asked_time_before_the_viewer_lands() {
        let paused = sample(TransportState::Stopped, 1_000_000, 3);
        let shown = scrub_shown(paused, false, Some(4.5));
        assert_eq!(
            shown.media_time_at(shown.host_time).get(),
            9 * TICKS_PER_SECOND / 2
        );
        assert_eq!(scrub_shown(paused, false, None), paused);
        let playing = sample(TransportState::Playing, 1_000_000, 3);
        assert_eq!(scrub_shown(playing, true, Some(4.5)), playing);
    }
}

/// Per-layer compilation failures are not viewer-wide errors: record them
/// before Properties points the person at the Error Log.
fn record_layer_failures(
    log: &mut crate::error_log::Log,
    failures: &[pm_render::LayerFailure],
    source: impl Fn(&pm_render::LayerFailure) -> String,
) {
    for failure in failures {
        let source = source(failure);
        if !log
            .entries
            .iter()
            .any(|entry| entry.source == source && entry.message == failure.message)
        {
            log.push(&source, &failure.message);
        }
    }
}
fn definition_failure_source(
    layer: &str,
    definition: &str,
    contributions: Option<&serde_json::Value>,
) -> String {
    let owner = contributions
        .and_then(|value| value["layers"].as_array())
        .and_then(|layers| layers.iter().find(|item| item["id"] == definition))
        .and_then(|item| item["owner"].as_str())
        .unwrap_or(definition);
    format!("Extension {owner} ({definition}), layer {layer}")
}
fn layer_failure_source(
    failure: &pm_render::LayerFailure,
    published: Option<&pm_kernel::Published>,
    contributions: Option<&serde_json::Value>,
) -> String {
    let layer = published.and_then(|p| {
        let root = p.snapshot.root()?;
        let comp = pm_kernel::model::comp_node(root, Some(&failure.composition)).unwrap_or(root);
        let at = pm_kernel::model::find_layer(comp, &failure.layer)?;
        Some(pm_kernel::model::layer_at(comp, &at)?.to_value())
    });
    match layer
        .as_ref()
        .and_then(|layer| layer.get("d"))
        .and_then(|data| data.get("definition"))
        .and_then(pm_model::Value::as_str)
    {
        Some(definition) => definition_failure_source(&failure.layer, definition, contributions),
        None => format!("Layer {}", failure.layer),
    }
}
#[cfg(test)]
mod extension_compile_log_tests {
    use super::{definition_failure_source, record_layer_failures};
    #[test]
    fn failed_extension_layers_log_the_owner_and_full_compiler_message_once() {
        let contributions =
            serde_json::json!({"layers":[{"id":"macos.cursor","owner":"macos-cursor"}]});
        let source = definition_failure_source("cursor-1", "macos.cursor", Some(&contributions));
        assert!(source.contains("macos-cursor"));
        let failures = vec![pm_render::LayerFailure {
            composition: "main".into(),
            layer: "cursor-1".into(),
            message: "shader compile failed\nline 33: cannot compare bool and float".into(),
        }];
        let mut log = crate::error_log::Log::default();
        for _ in 0..3 {
            record_layer_failures(&mut log, &failures, |_| source.clone());
        }
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].message, failures[0].message);
        assert_eq!(log.entries[0].source, source);
    }
}

#[cfg(test)]
mod placeholder_tests {
    /// A viewer bound again before the project's first frame (its document
    /// published again while it settles) keeps the stand-in picture; another
    /// document, or a poster staged for a new open, replaces it (PM-384).
    #[test]
    fn a_viewer_bound_again_keeps_its_placeholder() {
        assert!(super::placeholder_kept(Some(7), false, Some(7)));
        assert!(!super::placeholder_kept(Some(7), true, Some(7)), "a new open's poster");
        assert!(!super::placeholder_kept(Some(7), false, Some(8)), "another document");
        assert!(!super::placeholder_kept(None, false, Some(7)), "none shown");
    }
}

#[cfg(test)]
#[path = "cursor_render_tests.rs"]
mod cursor_render_tests;
