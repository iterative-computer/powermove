//! Layers, channels, and keyframes (`core/types/project.ts`).

use super::{Codec, JsMap, Nullable, record, string_enum};
use crate::json::{JsString, Map, Value};

/// A keyframeable value: `number | string | boolean`, and the number arrays
/// that 3D shader channels store (`shared/blender.ts`, `shaderValue`).
#[derive(Clone, Debug, PartialEq)]
pub enum ChannelValue {
    Number(f64),
    String(JsString),
    Bool(bool),
    Numbers(Vec<f64>),
}

impl Codec for ChannelValue {
    fn decode(value: Value) -> Result<Self, Value> {
        match value {
            Value::Number(n) => Ok(ChannelValue::Number(n)),
            Value::String(s) => Ok(ChannelValue::String(s)),
            Value::Bool(b) => Ok(ChannelValue::Bool(b)),
            Value::Array(_) => Vec::<f64>::decode(value).map(ChannelValue::Numbers),
            other => Err(other),
        }
    }

    fn encode(&self) -> Value {
        match self {
            ChannelValue::Number(n) => Value::Number(*n),
            ChannelValue::String(s) => Value::String(s.clone()),
            ChannelValue::Bool(b) => Value::Bool(*b),
            ChannelValue::Numbers(items) => items.encode(),
        }
    }
}

impl ChannelValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ChannelValue::Number(n) => Some(*n),
            _ => None,
        }
    }
}

string_enum! {
    /// After Effects temporal interpolation on one side of a key.
    pub enum Interpolation {
        Linear = "linear",
        Bezier = "bezier",
        Hold = "hold",
    }
}

record! {
    /// `TemporalEase`: speed in value units per second, influence in percent.
    pub struct TemporalEase {
        speed: f64 = "speed",
        influence: f64 = "influence",
    }
}

record! {
    /// Spring easing on a key (`legacy/core/anim.ts`).
    pub struct Spring {
        stiffness: f64 = "stiffness",
        damping: f64 = "damping",
        mass: f64 = "mass",
    }
}

record! {
    /// One key. `t` is seconds in layer time.
    pub struct Keyframe {
        t: f64 = "t",
        v: ChannelValue = "v",
        in_interp: Interpolation = "inInterp",
        out_interp: Interpolation = "outInterp",
        in_ease: TemporalEase = "inEase",
        out_ease: TemporalEase = "outEase",
        auto_bezier: bool = "autoBezier",
        continuous: bool = "continuous",
        /// The ease preset name.
        i: JsString = "i",
        spring: Spring = "spring",
        /// Legacy unit handles and hold flag, converted on load
        /// (`temporal-ease.ts`); present in older files.
        eo: Vec<f64> = "eo",
        ei: Vec<f64> = "ei",
        hold: bool = "hold",
    }
}

record! {
    /// `{ v, kf, expr }`: a static value, keys, and an optional expression.
    pub struct Channel {
        v: ChannelValue = "v",
        kf: Vec<Keyframe> = "kf",
        expr: Nullable<JsString> = "expr",
    }
}

/// A property stored either as a constant or, once keyframed or given an
/// expression, as a [`Channel`] (`PM.allProps` animates content fields, `blend`,
/// `mblur`, `matteMode`, effect and mask switches this way).
#[derive(Clone, Debug, PartialEq)]
pub enum Prop<T> {
    Static(T),
    Animated(Channel),
}

impl<T> Prop<T> {
    /// The constant, when the property is not a channel.
    pub fn as_static(&self) -> Option<&T> {
        match self {
            Prop::Static(v) => Some(v),
            Prop::Animated(_) => None,
        }
    }

    /// The channel, when the property is animated.
    pub fn as_channel(&self) -> Option<&Channel> {
        match self {
            Prop::Static(_) => None,
            Prop::Animated(c) => Some(c),
        }
    }
}

impl<T: Codec> Codec for Prop<T> {
    fn decode(value: Value) -> Result<Self, Value> {
        match value {
            Value::Object(_) => Channel::decode(value).map(Prop::Animated),
            other => T::decode(other).map(Prop::Static),
        }
    }

    fn encode(&self) -> Value {
        match self {
            Prop::Static(v) => v.encode(),
            Prop::Animated(c) => c.encode(),
        }
    }

    fn mismatches(&self, path: &mut String, out: &mut Vec<String>) {
        match self {
            Prop::Static(v) => v.mismatches(path, out),
            Prop::Animated(c) => c.mismatches(path, out),
        }
    }
}

string_enum! {
    /// `BLEND_MODES`.
    pub enum BlendMode {
        Normal = "normal",
        Add = "add",
        Screen = "screen",
        Multiply = "multiply",
        Overlay = "overlay",
        SoftLight = "softlight",
        Difference = "difference",
        Lighten = "lighten",
        Darken = "darken",
    }
}

string_enum! {
    /// `LayerType` (the keys of `TYPE_META`).
    pub enum LayerType {
        Solid = "solid",
        Text = "text",
        Shape = "shape",
        Image = "image",
        Video = "video",
        Audio = "audio",
        Adjustment = "adjustment",
        Shader = "shader",
        Extension = "extension",
        Null = "null",
        Group = "group",
        Precomp = "precomp",
        Captions = "captions",
    }
}

record! {
    /// An effect instance.
    pub struct Effect {
        id: JsString = "id",
        effect_type: JsString = "type",
        on: Prop<bool> = "on",
        /// Added when an effect is inserted through `Edit.add_effect`.
        open: bool = "open",
        p: JsMap<Channel> = "p",
        /// Marked by hydrate when the effect's extension is unavailable.
        missing: bool = "missing",
    }
}

record! {
    /// A layer's in or out transition.
    pub struct Transition {
        transition_type: JsString = "type",
        dur: f64 = "dur",
        p: JsMap<Channel> = "p",
        /// Marked by hydrate when the definition is unavailable.
        missing: bool = "missing",
    }
}

record! {
    /// One vertex of an editable path; channels `x y inX inY outX outY`.
    pub struct PathVertex {
        id: JsString = "id",
        p: JsMap<Channel> = "p",
    }
}

record! {
    /// An editable vector path (shape paths and mask paths).
    pub struct VectorPath {
        id: JsString = "id",
        name: JsString = "name",
        parent: Nullable<JsString> = "parent",
        vertices: Vec<PathVertex> = "vertices",
        p: JsMap<Channel> = "p",
    }
}

record! {
    /// A mask; channels `x y w h rotation feather`.
    pub struct Mask {
        id: JsString = "id",
        path: VectorPath = "path",
        shape: Prop<JsString> = "shape",
        mode: Prop<JsString> = "mode",
        on: Prop<bool> = "on",
        p: JsMap<Channel> = "p",
    }
}

record! {
    /// A text animator.
    pub struct TextAnimator {
        id: JsString = "id",
        name: JsString = "name",
        p: JsMap<Channel> = "p",
        enabled: bool = "enabled",
        /// `stagger | range | wave`; absent on animators saved before modes.
        mode: JsString = "mode",
        /// `characters | words | lines`; older animators use a `unit` channel.
        unit: JsString = "unit",
        order: JsString = "order",
        seed: f64 = "seed",
        direction: JsString = "direction",
        easing: JsString = "easing",
        shape: JsString = "shape",
        unit_offset: f64 = "unitOffset",
        unit_total: f64 = "unitTotal",
    }
}

record! {
    /// A styled character range of a text layer.
    pub struct TextStyleRange {
        id: JsString = "id",
        start: f64 = "start",
        end: f64 = "end",
        p: JsMap<Channel> = "p",
    }
}

record! {
    /// Font anchor bounds cached on text layers.
    pub struct FontAnchorBounds {
        x0: f64 = "x0",
        y0: f64 = "y0",
        x1: f64 = "x1",
        y1: f64 = "y1",
    }
}

record! {
    /// A layer's content (`d`). One record models the fields of every layer
    /// type; which apply depends on [`Layer::layer_type`]:
    ///
    /// | Type | Fields |
    /// |---|---|
    /// | solid, null | `color w h radius` |
    /// | text | `text boxWidth boxHeight font weight size tracking leading color align italic animators styles fontAxis fontAnchorBounds paragraph` |
    /// | shape | `shape color w h radius independentCorners radiusTL radiusTR radiusBR radiusBL smoothing stroke strokeColor points paths svgWidth svgHeight svgSourceAsset` |
    /// | image | `asset fit w h` |
    /// | video | image fields, `trim speed embeddedAudio audioMuted audioGain`, additive `pitch alwaysOriginal` |
    /// | audio | `asset gain trim fadeIn fadeOut`, additive `pitch alwaysOriginal` |
    /// | shader | `code w h uniforms` |
    /// | extension | `definition version w h params data` (3D layers: [`ExtensionData`]) |
    /// | group | `modeling` (model groups) |
    /// | precomp | `comp w h trim timeRemap sourceTime audioGain audioMuted`, additive `keepFrameRate keepRendered` |
    /// | captions | `cues style language` |
    ///
    /// Content fields may be animated ([`Prop`]).
    pub struct LayerContent {
        // Shared.
        color: Prop<JsString> = "color",
        w: Prop<f64> = "w",
        h: Prop<f64> = "h",
        radius: Prop<f64> = "radius",
        // Text.
        text: Prop<JsString> = "text",
        box_width: Prop<f64> = "boxWidth",
        box_height: Prop<f64> = "boxHeight",
        font: Prop<JsString> = "font",
        weight: Prop<f64> = "weight",
        size: Prop<f64> = "size",
        tracking: Prop<f64> = "tracking",
        leading: Prop<f64> = "leading",
        align: Prop<JsString> = "align",
        italic: Prop<bool> = "italic",
        animators: Vec<TextAnimator> = "animators",
        styles: Vec<TextStyleRange> = "styles",
        font_axis: JsMap<Prop<f64>> = "fontAxis",
        font_anchor_bounds: Nullable<FontAnchorBounds> = "fontAnchorBounds",
        paragraph: Prop<bool> = "paragraph",
        // Shape.
        shape: Prop<JsString> = "shape",
        independent_corners: Prop<bool> = "independentCorners",
        radius_tl: Prop<f64> = "radiusTL",
        radius_tr: Prop<f64> = "radiusTR",
        radius_br: Prop<f64> = "radiusBR",
        radius_bl: Prop<f64> = "radiusBL",
        smoothing: Prop<f64> = "smoothing",
        stroke: Prop<f64> = "stroke",
        stroke_color: Prop<JsString> = "strokeColor",
        points: Prop<f64> = "points",
        paths: Vec<VectorPath> = "paths",
        svg_width: f64 = "svgWidth",
        svg_height: f64 = "svgHeight",
        svg_source_asset: JsString = "svgSourceAsset",
        // Image, video, audio.
        asset: Nullable<JsString> = "asset",
        fit: Prop<JsString> = "fit",
        /// Seconds into the source (video, audio); a channel on precomps.
        trim: Prop<f64> = "trim",
        speed: Prop<f64> = "speed",
        embedded_audio: bool = "embeddedAudio",
        audio_muted: Prop<bool> = "audioMuted",
        /// Linear level 0 to 4, a number or a channel (video, precomp).
        audio_gain: Prop<f64> = "audioGain",
        gain: Prop<f64> = "gain",
        fade_in: Prop<f64> = "fadeIn",
        fade_out: Prop<f64> = "fadeOut",
        /// Additive (section 6): preserve pitch or follow speed. Its shape is
        /// defined with the audio engine (Phase 3b); modelled as raw JSON.
        pitch: Value = "pitch",
        /// Additive (section 6): Always Play Original.
        always_original: bool = "alwaysOriginal",
        // Shader.
        code: JsString = "code",
        uniforms: JsMap<Channel> = "uniforms",
        // Extension.
        definition: JsString = "definition",
        version: f64 = "version",
        params: JsMap<Channel> = "params",
        data: ExtensionData = "data",
        // Group.
        modeling: super::GeneratedModel = "modeling",
        // Precomp.
        comp: Nullable<JsString> = "comp",
        time_remap: Prop<bool> = "timeRemap",
        source_time: Prop<f64> = "sourceTime",
        /// Additive (section 6): keep the child's frame rate when nested.
        keep_frame_rate: bool = "keepFrameRate",
        /// Additive (section 6): a kept render. Its shape is defined with the
        /// render cache (Phase 4b); modelled as raw JSON.
        keep_rendered: Value = "keepRendered",
        // Captions.
        cues: Vec<super::CaptionCue> = "cues",
        style: super::CaptionStyle = "style",
        language: JsString = "language",
    }
}

record! {
    /// `d.data` of an extension layer. Free-form for most extensions; the 3D
    /// layers store their content here (`core/scene3d/layers.ts`), and legacy
    /// scene layers a whole [`super::Scene3D`].
    pub struct ExtensionData {
        object: super::ObjectContent = "object",
        light: super::LightContent = "light",
        camera: super::CameraContent = "camera",
        environment: super::Environment = "environment",
        scene: super::Scene3D = "scene",
    }
}

record! {
    /// One layer. Times are seconds; `from` is composition time, key times are
    /// layer time.
    pub struct Layer {
        id: JsString = "id",
        layer_type: LayerType = "type",
        name: JsString = "name",
        from: f64 = "from",
        dur: f64 = "dur",
        on: Prop<bool> = "on",
        lock: bool = "lock",
        shy: bool = "shy",
        matte_source: Nullable<JsString> = "matteSource",
        matte_mode: Prop<JsString> = "matteMode",
        solo: bool = "solo",
        collapsed: bool = "collapsed",
        scale_linked: bool = "scaleLinked",
        three_d: bool = "threeD",
        color: JsString = "color",
        blend: Prop<BlendMode> = "blend",
        mblur: Prop<bool> = "mblur",
        group: Nullable<JsString> = "group",
        /// Preferred timeline lane relative to the layer's group.
        track: f64 = "track",
        parent: Nullable<JsString> = "parent",
        /// Transform channels by flat name (`position.x`, `opacity`, ...).
        p: JsMap<Channel> = "p",
        fx: Vec<Effect> = "fx",
        transition_in: Nullable<Transition> = "transitionIn",
        transition_out: Nullable<Transition> = "transitionOut",
        masks: Vec<Mask> = "masks",
        d: LayerContent = "d",
        locked_intent: Map = "locked_intent",
        /// Additive (section 6): 1 for layers created before cutover, 2 after.
        compat: f64 = "compat",
    }
}
