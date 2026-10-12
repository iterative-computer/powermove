//! The project, compositions, assets, and the save envelope
//! (`core/types/project.ts`, section 6's additive fields).

use super::{JsMap, Layer, Render3D, record, string_enum};
use crate::json::{JsString, Map, Value};

record! {
    /// A gradient stop.
    pub struct FillStop {
        id: JsString = "id",
        color: JsString = "color",
        position: f64 = "position",
    }
}

record! {
    /// A background fill (`type`: `solid linear radial none`).
    pub struct Fill {
        fill_type: JsString = "type",
        angle: f64 = "angle",
        stops: Vec<FillStop> = "stops",
    }
}

record! {
    /// A timeline marker. Legacy demo markers have no `id`.
    pub struct Marker {
        id: JsString = "id",
        t: f64 = "t",
        name: JsString = "name",
    }
}

record! {
    /// An option of a `select` scene parameter.
    pub struct SceneSelectOption {
        v: Value = "v",
        label: JsString = "label",
    }
}

record! {
    /// A scene parameter (`control`: `num color toggle select`).
    pub struct SceneParam {
        name: JsString = "name",
        label: JsString = "label",
        control: JsString = "control",
        value: Value = "value",
        min: f64 = "min",
        max: f64 = "max",
        options: Vec<SceneSelectOption> = "options",
    }
}

record! {
    /// One edit-log entry.
    pub struct EditLogEntry {
        id: JsString = "id",
        revision: f64 = "revision",
        at: f64 = "at",
        origin: JsString = "origin",
        label: JsString = "label",
        summary: Vec<JsString> = "summary",
        operations: Vec<Map> = "operations",
        payload_omitted: bool = "payloadOmitted",
    }
}

record! {
    /// Per-project export defaults (`core/export-defaults.ts`).
    pub struct ExportDefaults {
        format: JsString = "format",
        scale: f64 = "scale",
        fps: f64 = "fps",
        range: JsString = "range",
        quality: JsString = "quality",
        mblur: bool = "mblur",
        audio: bool = "audio",
        alpha: bool = "alpha",
    }
}

string_enum! {
    /// Additive asset `storage` (section 21).
    pub enum AssetStorage {
        Copied = "copied",
        Linked = "linked",
    }
}

record! {
    /// Asset metadata (`assetIdentity` in `legacy/gl/raster.ts`); free-form in
    /// the TypeScript, so everything else stays in `extra`.
    pub struct Asset {
        id: JsString = "id",
        name: JsString = "name",
        /// `image video audio svg model ...`.
        kind: JsString = "kind",
        fingerprint: Value = "fingerprint",
        storage_key: JsString = "storageKey",
        source_path: JsString = "sourcePath",
        size: f64 = "size",
        dur: f64 = "dur",
        w: f64 = "w",
        h: f64 = "h",
        /// A hint from the file (ingested as it is, section 6).
        fps: f64 = "fps",
        channels: f64 = "channels",
        sample_rate: f64 = "sampleRate",
        playback_proxy: bool = "playbackProxy",
        playback_proxy_version: f64 = "playbackProxyVersion",
        persisted: bool = "persisted",
        image_sequence: Value = "imageSequence",
        poster: bool = "poster",
        /// Ingested as it is (section 6).
        format: JsString = "format",
        editable_paths: f64 = "editablePaths",
        has_audio: bool = "hasAudio",
        audio_dur: f64 = "audioDur",
        vertices: f64 = "vertices",
        triangles: f64 = "triangles",
        layer_definition: JsString = "layerDefinition",
        folder: JsString = "folder",
        /// Additive (section 6): a linked-media reference. Its shape is defined
        /// with the media store (Phase 2); modelled as raw JSON.
        link: Value = "link",
        /// Additive (section 6): interpretation overrides (Phase 3b); raw JSON.
        interpret: Value = "interpret",
        /// Additive (section 6): `copied` or `linked`.
        storage: AssetStorage = "storage",
    }
}

string_enum! {
    /// Additive root `requiresEngine` (section 6).
    pub enum RequiresEngine {
        Native = "native",
    }
}

record! {
    /// A composition. The project root is a composition too (TypeScript
    /// `Project extends Comp`): in the stored form it holds the open
    /// composition's fields plus `compId` and `compName`, and `comps` holds the
    /// others ([`crate::canonical`]). Nested compositions are made by
    /// `mkProject` and may carry every member.
    ///
    /// Root-only members: `compId`, `compName`, `exportDefaults`, `library`,
    /// `notes`, and the additive `minReaderVersion`, `requiresEngine`, `color`,
    /// `media`.
    pub struct Composition {
        id: JsString = "id",
        name: JsString = "name",
        w: f64 = "w",
        h: f64 = "h",
        /// The float rate as stored; see [`Composition::fps_rational`].
        fps: f64 = "fps",
        dur: f64 = "dur",
        bg: JsString = "bg",
        background_fill: Fill = "backgroundFill",
        layers: Vec<Layer> = "layers",
        comps: JsMap<Composition> = "comps",
        assets: JsMap<Asset> = "assets",
        markers: Vec<Marker> = "markers",
        work: [f64; 2] = "work",
        params: JsMap<SceneParam> = "params",
        revision: f64 = "revision",
        edits: Vec<EditLogEntry> = "edits",
        created: f64 = "created",
        shutter: f64 = "shutter",
        audio_gain: f64 = "audioGain",
        /// Ingested as it is (section 6).
        render3d: Render3D = "render3d",
        export_defaults: ExportDefaults = "exportDefaults",
        library: Map = "library",
        notes: JsString = "notes",
        comp_id: JsString = "compId",
        comp_name: JsString = "compName",
        /// Additive (section 6): the exact rate `[num, den]`.
        fps_rational: [f64; 2] = "fpsRational",
        /// Additive (section 6): motion blur samples per output kind. Its shape
        /// is defined with motion blur (Phase 3a); modelled as raw JSON.
        motion_blur_samples: Value = "motionBlurSamples",
        /// Additive (section 6): the project audio rate in hertz.
        audio_sample_rate: f64 = "audioSampleRate",
        /// Additive (section 6, the Phase 0 guard): the lowest reader version
        /// that understands this project.
        min_reader_version: Value = "minReaderVersion",
        /// Additive (section 6): set when a native-only feature is used.
        requires_engine: RequiresEngine = "requiresEngine",
        /// Additive (section 6): project color settings (Phase 3a); raw JSON.
        color: Value = "color",
        /// Additive (section 6): proxy and import settings (Phase 4a); raw JSON.
        media: Value = "media",
    }
}

/// The project root.
pub type Project = Composition;

impl Composition {
    /// `fpsRational` as integers, when it holds two safe integers.
    pub fn fps_rational(&self) -> Option<[i64; 2]> {
        let [num, den] = self.fps_rational?;
        let int =
            |x: f64| (x.fract() == 0.0 && x.abs() <= 9_007_199_254_740_991.0).then_some(x as i64);
        Some([int(num)?, int(den)?])
    }
}

record! {
    /// The saved document inside a project file: `{ v, proj, history, ws }`.
    /// Older files store a bare project instead
    /// ([`crate::canonical::project_of`]). `minReaderVersion` may also be
    /// stamped here (`shared/reader-version.ts`).
    pub struct SavedDocument {
        v: f64 = "v",
        proj: Project = "proj",
        /// Undo history (`legacy/core/history.ts`), kept as raw JSON.
        history: Value = "history",
        /// Workspace state, kept as raw JSON.
        ws: Value = "ws",
        min_reader_version: Value = "minReaderVersion",
    }
}

record! {
    /// A web export scene (`format: "powermove-web"`), as shipped in the
    /// onboarding and www hero scenes.
    pub struct WebScene {
        format: JsString = "format",
        version: f64 = "version",
        project: Project = "project",
    }
}
