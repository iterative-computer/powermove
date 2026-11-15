//! Explicit turn undo, with the user's changes retained after confirmation.
use crate::{
    alert::{Alert, Button},
    session::Controller,
};
use gpui::Context;
use pm_kernel::{Caller, HistoryCall, Reply, Request};
impl Controller {
    pub(crate) fn undo_agent_turn(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.session.as_ref().map(|s| s.project.clone()) else {
            return;
        };
        let Some((writer, label)) = self.mcp.latest_turn(&project) else {
            return;
        };
        let kernel = self.kernel.clone();
        cx.spawn(async move |this,cx| {
            let request=|keep_mine| Request::new(Caller::native(),HistoryCall::UndoWriter {
                writer,keep_mine,label:format!("Undo Agent · {label}"),
            });
            let reply=kernel.call(&project,request(false)).await;
            let confirmation=matches!(&reply,Some(Ok(ack)) if matches!(ack.reply,Reply::Workspace(_)));
            if confirmation {
                let answer=this.update(cx, |s,_| {
                    Alert::new(
                        "You've changed things the agent made since.",
                        "Undo the rest of this agent turn and keep your changes?",
                        vec![Button::new("Keep Mine and Undo Rest",true).default(),Button::new("Cancel",false).cancel()],
                    ).sheet(&s.host.borrow().window)
                }).ok();
                let Some(answer)=answer else {return;};
                if answer.recv().await.ok().flatten()!=Some(true) {return;}
                let reply=kernel.call(&project,request(true)).await;
                let _=this.update(cx, |s,cx| {
                    s.message=if matches!(reply,Some(Ok(ref ack)) if matches!(ack.reply,Reply::Applied{..})) {
                        "Undid agent turn · kept your changes".into()
                    } else {"Couldn't undo the agent turn; your edits were preserved".into()};
                    cx.notify();
                });
            } else {
                let _=this.update(cx, |s,cx| {
                    s.message=if matches!(reply,Some(Ok(ref ack)) if matches!(ack.reply,Reply::Applied{..})) {
                        "Undid agent turn".into()
                    } else {"Couldn't undo the agent turn; finish the active edit first".into()};
                    cx.notify();
                });
            }
        }).detach();
    }
}

use gpui::{WindowKind, prelude::*, *};
use pm_shell::text::{TextEvent, TextField, TextKind};
struct Steer {
    field: Entity<TextField>,
    _events: Subscription,
    tokens: pm_design::Tokens,
}
impl Render for Steer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(6.))
            .bg(rgb(self.tokens.colors.floating))
            .child(self.field.clone())
    }
}
impl Controller {
    pub(crate) fn open_agent_steer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(runtime) = self.builtins.runtime.clone() else {
            return;
        };
        let Some(thread) = runtime.agent_current_thread() else {
            return;
        };
        if !runtime.agent_status().0 {
            return;
        }
        let tokens = self.host.borrow().tokens.clone();
        let bounds = window.bounds();
        let parent = window.window_handle();
        let native = self.host.borrow().window.clone();
        let responder = native.firstResponder();
        let origin = bounds.origin
            + point(
                px((f32::from(bounds.size.width) - 330.).max(0.)),
                px(tokens.metrics.toolbar + tokens.metrics.gap),
            );
        let owner = cx.entity().downgrade();
        cx.defer(move |cx| {
            let mut options=WindowOptions::new().kind(WindowKind::PopUp)
                .window_bounds(Some(WindowBounds::Windowed(Bounds::new(origin,size(px(320.),px(44.))))))
                .titlebar(None)
                .macos_window_background(gpui::MacosWindowBackground::Transparent);
            options.parent=Some(parent);
            let _=cx.open_window(options,move |window,cx| {
                pm_shell::popover_frame(window,&tokens);
                cx.new(|cx| {
                    let field=cx.new(|cx| {
                        let mut field=TextField::new(TextKind::Composer,"Tell the agent","",tokens.clone(),window,cx);
                        field.set_bare(true);field.set_placeholder("Tell the agent…");
                        field.begin_editing(cx);field
                    });
                    let events=cx.subscribe_in(&field,window,move |_:&mut Steer,_,event,window,cx| {
                        match event {
                            TextEvent::Submitted(text) if !text.trim().is_empty()=>{
                                pm_mcp::coedit::record_steer(runtime.project_id());
                                let (runtime,thread,message,owner)=(runtime.clone(),thread.clone(),text.trim().to_owned(),owner.clone());
                                cx.spawn(async move |_,cx| {
                                    let result=cx.background_executor().spawn(async move {
                                        runtime.agent_thread_control("thread_send",&serde_json::json!({"threadId":thread,"message":message,"callerAccess":"project"}))
                                    }).await;
                                    let _=owner.update(cx,|s,cx| {s.message=match result {Ok(_)=>"Sent steering".into(),Err(error)=>error};cx.notify();});
                                }).detach();
                                window.remove_window();
                            }
                            TextEvent::Key("escape")=>window.remove_window(),
                            _=>{}
                        }
                    });
                    cx.on_release(move |_,_| {native.makeFirstResponder(responder.as_deref());}).detach();
                    Steer{field,_events:events,tokens:tokens.clone()}
                })
            });
        });
    }
}
