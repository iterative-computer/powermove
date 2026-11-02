//! The Export extension owns its draft; native code owns destination sheets,
//! frozen snapshots and the real engine child, including cancellation/reaping.
use crate::session::Controller;
use gpui::Context;
use pm_protocol::{
    color::{OutputColor, Precision},
    common::{FrameRate, SessionId, Size, TICKS_PER_SECOND, TickRange, Ticks},
    priority::{ExportSettings, JobSpec, JobState},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
pub type Job = Arc<Mutex<Option<pm_engine_service::jobs::ExportHandle>>>;
impl Controller {
    pub fn begin_builtin_export(&mut self, options: Value, cx: &mut Context<Self>) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        if self
            .builtins
            .export_job
            .as_ref()
            .is_some_and(|j| j.lock().unwrap().is_some())
        {
            return;
        }
        let Some(p) = self.published.clone() else {
            return;
        };
        let typed = match serde_json::from_value::<pm_promedia::export_panel::Options>(options) {
            Ok(o) => o,
            Err(e) => {
                let _ = runtime.observe_export(json!({"phase":"failed","message":e.to_string()}));
                return;
            }
        };
        let project = p.project.clone();
        let document = p.snapshot.doc_id;
        let filename = format!(
            "{}.{}",
            p.snapshot
                .root()
                .and_then(|r| r.get("name"))
                .and_then(crate::node_string)
                .unwrap_or("Untitled"),
            typed.format.extension()
        );
        let picker = crate::platform::paths_named(&self.host.borrow().window, true, &filename);
        let media = self.media.clone();
        let activity = self.engine.export_activity();
        let kernel = self.kernel.clone();
        let duration = self.duration;
        let playhead = self.time();
        let job: Job = Arc::new(Mutex::new(None));
        self.builtins.export_job = Some(job.clone());
        cx.spawn(async move|this,cx|{
            let Ok(Some(destination))=picker.recv().await else{let _=runtime.observe_export(json!({"phase":"idle"}));return;};
            let Some(published)=kernel.published(&project).filter(|p|p.snapshot.doc_id==document)else{let _=runtime.observe_export(json!({"phase":"failed","message":"The project is no longer open."}));return;};
            let start_job=cx.background_executor().spawn(async move { (||->anyhow::Result<_>{
                use pm_protocol::validate::Validate;
                let options=typed;
                let root=published.snapshot.root().ok_or_else(||anyhow::anyhow!("Project closed"))?;
                let comp=pm_kernel::model::comp_node(root,published.snapshot.open_comp().as_deref()).ok_or_else(||anyhow::anyhow!("No composition"))?;
                let work=comp.get("work").map(|n|n.to_value());
                let (from,to)=if options.format==pm_promedia::export_panel::Format::Still{(playhead,playhead+1./options.fps)}else if options.range=="work"{let a=work.as_ref().and_then(|v|v.as_array());(a.and_then(|v|v.first()).and_then(|v|v.as_f64()).unwrap_or(0.),a.and_then(|v|v.get(1)).and_then(|v|v.as_f64()).unwrap_or(duration))}else{(0.,duration)};
                if options.format==pm_promedia::export_panel::Format::Json{anyhow::bail!("Portable project delivery requires the original-media writer");}
                let layers=pm_kernel::model::list_layers(root,published.snapshot.open_comp().as_deref());
                let intent=options.intent(layers.iter().any(|l|matches!(l.kind.as_str(),"audio"|"video")),0).map_err(anyhow::Error::msg)?;
                let rate=pm_time::rate_from_document(None,options.fps)?.rate;
                let base=ExportSettings{format:pm_protocol::priority::ExportFormat::H264,alpha:pm_protocol::priority::AlphaOutput::None,quality:pm_protocol::priority::QualityTier::High,size:Size::new(options.width,options.height),frame_rate:FrameRate::new(rate.num() as u32,rate.den() as u32),range:TickRange{start:Ticks::new((from*TICKS_PER_SECOND as f64).round() as i64)?,end:Ticks::new((to*TICKS_PER_SECOND as f64).round() as i64)?},audio:false,audio_index_playhead:None,motion_blur:false,color:OutputColor::SrgbImage,precision:Precision::Float16,dither:true,draft_from_proxies:false,delivery:Default::default()};
                let settings=intent.engine_settings(base).map_err(anyhow::Error::msg)?.ok_or_else(||anyhow::anyhow!("Portable project is not a render job"))?;settings.validate()?;
                let frames=((to-from)*options.fps).ceil().max(1.) as u64;
                let request=pm_engine_service::jobs::ExportRequest::new(pm_kernel::new_id("export").as_str().into(),&published.snapshot,JobSpec::Export{session:SessionId::new(document).unwrap(),composition:published.snapshot.open_comp().unwrap_or_default().as_str().into(),settings},media,destination)?;
                let handle=pm_engine_service::jobs::ExportHandle::start_scheduled(&std::env::current_exe()?,request,activity)?;
                Ok((handle,frames))
            })() }).await;
            let (handle,total)=match start_job{Ok(v)=>v,Err(error)=>{let _=runtime.observe_export(json!({"phase":"failed","message":error.to_string()}));return;}};
            *job.lock().unwrap()=Some(handle);let started=std::time::Instant::now();
            loop{
                let status=job.lock().unwrap().as_ref().map(|j|j.status());let Some(status)=status else{break;};
                let run=match status.state{JobState::Done=>json!({"phase":"done","path":status.path.clone().unwrap_or_default()}),JobState::Cancelled=>json!({"phase":"cancelled"}),JobState::Failed=>json!({"phase":"failed","message":status.error.as_ref().map(|e|e.message.clone()).unwrap_or_default()}),JobState::Running=>json!({"phase":"rendering","frame":(status.progress*total as f64).round() as u64,"total":total,"elapsed":started.elapsed().as_secs_f64()}),_=>json!({"phase":"preparing"})};
                let terminal=matches!(status.state,JobState::Done|JobState::Cancelled|JobState::Failed);
                if terminal{
                    let handle=job.lock().unwrap().take();
                    if let Some(handle)=handle{let _=cx.background_executor().spawn(async move{handle.wait()}).await;}
                }
                if let Ok(ticket)=runtime.observe_export(run){let _=ticket.await;}
                if terminal{break;}
                cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
            }
            let _=this.update(cx,|_,cx|cx.notify());
        }).detach();
    }
}
