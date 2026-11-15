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
