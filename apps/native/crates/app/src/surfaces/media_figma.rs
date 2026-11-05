use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clipboard {
    Supported,
    Fallback { reasons: Vec<String>, preview: bool },
    Unreadable,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presentation {
    PasteEditable,
    Alert(Alert),
    Banner(Banner),
}
pub fn present(target: Target, state: Clipboard) -> Presentation {
    match state {
        Clipboard::Supported => Presentation::PasteEditable,
        Clipboard::Unreadable => Presentation::Banner(Banner {
            text: "Couldn’t read the Figma selection. Copy it again in Figma.".into(),
            severity: Severity::Warning,
            action: None,
        }),
        Clipboard::Fallback { reasons, preview } => {
            let mut lines = reasons.iter().take(4).cloned().collect::<Vec<_>>();
            if reasons.len() > 4 {
                lines.push(format!("and {} more", reasons.len() - 4));
            }
            lines.push(if preview { "The whole selection becomes one image you can animate; its parts won’t be editable." } else { "Figma didn’t include a preview image. Use Copy as PNG in Figma, then paste again." }.into());
            Presentation::Alert(Alert {
                target,
                message: if preview {
                    "Paste the Figma selection as an image?"
                } else {
                    "These Figma layers need an image fallback."
                }
                .into(),
                informative: lines.join(" "),
                buttons: if preview {
                    vec![Action::PasteImage, Action::Cancel]
                } else {
                    vec![Action::Ok]
                },
                suppression: None,
            })
        }
    }
}
/// Converter supplies classified approved short reasons, never raw errors.
/// The entire selection is pasted in one operation after explicit consent.
pub fn respond(target: Target, state: &Clipboard, action: Action) -> Option<Command> {
    (action == Action::PasteImage && matches!(state, Clipboard::Fallback { preview: true, .. }))
        .then_some(Command {
            target,
            operation: Operation::PasteFigmaImage,
        })
}
