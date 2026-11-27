//! Captions layer content (`core/types/captions.ts`). Cue and word times are
//! seconds in layer time.

use super::{record, string_enum};
use crate::json::JsString;

record! {
    /// A timed word from transcription.
    pub struct CaptionWord {
        text: JsString = "text",
        start: f64 = "start",
        end: f64 = "end",
    }
}

record! {
    /// One cue. A newline in `text` is an authored line break.
    pub struct CaptionCue {
        id: JsString = "id",
        start: f64 = "start",
        end: f64 = "end",
        text: JsString = "text",
        words: Vec<CaptionWord> = "words",
    }
}

string_enum! {
    /// `CAPTION_PLACEMENTS`.
    pub enum CaptionPlacement {
        Bottom = "bottom",
        Middle = "middle",
        Top = "top",
    }
}

string_enum! {
    /// `CAPTION_TEXT_CASES`.
    pub enum CaptionTextCase {
        Original = "original",
        Upper = "upper",
        Lower = "lower",
    }
}

string_enum! {
    /// `CAPTION_WORD_ANIMATIONS`.
    pub enum CaptionWordAnimation {
        None = "none",
        Pop = "pop",
        Fade = "fade",
    }
}

string_enum! {
    /// `CAPTION_ALIGNS`.
    pub enum CaptionAlign {
        Left = "left",
        Center = "center",
        Right = "right",
    }
}

record! {
    /// `CaptionStyle`. Sizes are pixels at the composition's resolution.
    pub struct CaptionStyle {
        preset: JsString = "preset",
        font: JsString = "font",
        weight: f64 = "weight",
        italic: bool = "italic",
        size: f64 = "size",
        tracking: f64 = "tracking",
        leading: f64 = "leading",
        fill: JsString = "fill",
        stroke: f64 = "stroke",
        stroke_color: JsString = "strokeColor",
        r#box: bool = "box",
        box_color: JsString = "boxColor",
        box_opacity: f64 = "boxOpacity",
        box_padding: f64 = "boxPadding",
        box_radius: f64 = "boxRadius",
        placement: CaptionPlacement = "placement",
        align: CaptionAlign = "align",
        offset_x: f64 = "offsetX",
        offset_y: f64 = "offsetY",
        safe_margin: f64 = "safeMargin",
        max_width: f64 = "maxWidth",
        max_lines: f64 = "maxLines",
        text_case: CaptionTextCase = "textCase",
        highlight: bool = "highlight",
        highlight_color: JsString = "highlightColor",
        highlight_scale: f64 = "highlightScale",
        word_animation: CaptionWordAnimation = "wordAnimation",
    }
}
