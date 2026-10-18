//! One valid sample envelope for every message kind.
//!
//! Used by the round-trip tests, by the golden wire fixtures in
//! `tests/fixtures/` (which the TypeScript wire test decodes too), and as
//! worked examples for the RFCs. [`all`] is checked for exhaustiveness: a
//! new request, response, or event variant fails the build of
//! [`request_index`], [`response_index`], or [`event_index`] until it has a
//! sample.

use crate::blender::{
    BlenderAsset, BlenderEngine, BlenderJobRequest, BlenderModelJob, BlenderModelOperation,
    BlenderOutputFile, BlenderProgress, BlenderSettings, BlenderStatus, BlenderSurface,
    RequestedDevice, ResolvedDevice, ShutterSample, SnapshotEncoding, SnapshotRef,
    SurfaceRequest as BlenderSurfaceRequest,
};
use crate::clock::{
    AudioLevels, AudioMessage, AudioPeaks, ChannelLevels, ClockSample, SeekMode, StripLevels,
    TransportCommand, TransportState,
};
use crate::color::{
    AlphaMode, ChromaSiting, ColorDescription, MatrixCoefficients, OutputColor, Precision,
    Primaries, ProjectColor, Range, Subsampling, ToneMapping3d, Transfer, ViewerColorSpace,
};
use crate::common::{
    ByteSize, Bytes, ContentHash, Count, FrameRate, HostMicros, MAT3_IDENTITY, MAT4_IDENTITY,
    MAX_SAFE_INTEGER, PixelRect, PlaybackRate, Rect, RequestId, Revision, Rgba, Seq, SessionId,
    Size, TickRange, Ticks,
};
use crate::definitions::{
    DefinitionFlags, DefinitionKind, DefinitionOrigin, DefinitionRemoval, DefinitionSnapshot,
    DefinitionsApplied, DefinitionsUpdate, Layer3dRole, LayerRenderer, ParamSpec, WrappedGlsl,
};
use crate::document_host::{
    CloseProject, ExtensionRegistered, ExtensionSource, InstanceId, OpenProject, Persist,
    PersistReason, Persisted, ProjectOpened, RegisterExtension, ToolCall, ToolResult,
};
use crate::envelope::{
    Envelope, EvaluateRequest, Event, Message, OpenSession, RenderedImage, Request, Response,
    SessionRole,
};
use crate::error::{
    CrashReport, EngineError, EngineProcess, ErrorCode, ErrorDetail, ErrorScope, FailureReport,
    InFlight, Retry, ShaderDiagnostic,
};
use crate::frame::{
    Caret, DepthIdSurface, EditorCamera, FrameDescriptor, FramePurpose, FrameQuality, FrameRelease,
    FrameRequest, MediaPreference, MotionBlurKind, OverlayGeometry, OverlayLayer, PathCommand,
    PreviewResolution, Projection, Representation, Shading3d, ViewOverride, ViewerState, VsyncHint,
};
use crate::host::{
    CacheKind, CapabilityHandle, CapabilityKind, CommitOutput, HandleAccess, HandleGrant,
    HandleResolved, HardwareSessionGrant, HardwareSessionKind, HardwareSessionRequest, MintHandle,
    OutputChoice, OutputCommitted, OutputDestination, OutputDestinationRequest, OutputVerification,
    OverwritePolicy, ResolveHandle,
};
use crate::interop::{PlatformSurface, SurfaceDescriptor, SurfaceFormat, SurfacePool};
use crate::lease::{Lease, LeaseCategories, LeaseReason, LeaseRequest, LeaseUsage};
use crate::patch::{
    ConsistencyCheck, ConsistencyResult, DocumentEncoding, EngineFact, PatchBatch, PatchEncoding,
    PatchOrigin, PatchesApplied, RelinkReason, Resync, ResyncSource,
};
use crate::priority::{
    AgentFrame, AgentFrameOutcome, AgentFrameRequest, AgentFrameResult, AlphaOutput, AudioCoding,
    BlenderUse, CancelJobs, DeliveryOptions, ExportFormat, ExportSettings, ExportTimecode,
    ImagePayload, JobAccepted, JobPreview, JobSpec, JobState, JobStatus, JobsCancelled,
    PriorityClass, ProResProfile, QualityTier, SubmitJob,
};
use crate::scene::{
    Background, Camera3d, Compat, DepthSet, EvaluatedEffect, EvaluatedLayer, EvaluatedMask,
    EvaluatedScene, EvaluatedTransition, GradientStop, LayerKind, LayerSource, LayerTransform,
    Light3d, LightKind, MaskMode, MatteMode, MatteRef, ModelSource, NamedParam, Object3d,
    OrderedRun, ParamValue, Shutter, World3d,
};
use crate::slicing::{BlenderThrottle, DeadlineSignal, SlicingPolicy};
use crate::version::{ContractVersion, Hello, PeerRole, WIRE_VERSION, Welcome, local_ranges};

fn handle(n: u8) -> CapabilityHandle {
    CapabilityHandle::from_random_bytes([n; 16])
}

fn hash(n: u8) -> ContentHash {
    ContentHash([n; 32])
}

fn rev(n: u64) -> Revision {
    Revision::new(n).expect("sample revision")
}

fn seq(n: u64) -> Seq {
    Seq::new(n).expect("sample seq")
}

fn session() -> SessionId {
    SessionId::new(7).expect("sample session")
}

fn bytes(n: u64) -> ByteSize {
    ByteSize::new(n).expect("sample size")
}

fn ticks(n: i64) -> Ticks {
    Ticks::new(n).expect("sample ticks")
}

fn request(id: u64, body: Request) -> Envelope {
    let bound = body.session_bound();
    let envelope = Envelope::new(Message::Request {
        id: RequestId::new(id).expect("sample id"),
        body,
    });
    if bound {
        envelope.with_session(session())
    } else {
        envelope
    }
}

fn response(id: u64, body: Response) -> Envelope {
    Envelope::new(Message::Response {
        id: RequestId::new(id).expect("sample id"),
        body,
    })
    .with_revision(rev(42))
}

fn event(body: Event) -> Envelope {
    let bound = body.session_bound();
    let envelope = Envelope::new(Message::Event { body }).with_revision(rev(42));
    if bound {
        envelope.with_session(session())
    } else {
        envelope
    }
}

pub fn project_color() -> ProjectColor {
    ProjectColor::DEFAULT
}

pub fn view_override() -> ViewOverride {
    ViewOverride {
        camera: Some(EditorCamera {
            view: MAT4_IDENTITY,
            projection: Projection::Orthographic {
                half_height: 3.0,
                near: 0.01,
                far: 200.0,
            },
        }),
        shading: Shading3d::Solid,
        xray: true,
        local_view: vec!["cube".into()],
    }
}

pub fn viewer_frame_request() -> FrameRequest {
    FrameRequest {
        composition: "main".into(),
        time: ticks(-29_429_400),
        purpose: FramePurpose::Viewer,
        priority: PriorityClass::Interactive,
        output_size: Size::new(3840, 2160),
        region: Some(PixelRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        }),
        scale: 1.0,
        resolution: PreviewResolution::Automatic,
        media: MediaPreference::Automatic,
        color: OutputColor::Viewer {
            space: ViewerColorSpace::DisplayP3,
        },
        precision: Precision::Float16,
        motion_blur: MotionBlurKind::PreviewPaused,
        view: Some(view_override()),
        draft_3d: true,
        draft_render: false,
        depth_id: true,
        overlay: true,
    }
}

pub fn frame_descriptor() -> FrameDescriptor {
    FrameDescriptor {
        pool_index: 3,
        pool_generation: seq(2),
        seq: seq(MAX_SAFE_INTEGER as u64),
        pts: ticks(29_429_400),
        revision: rev(42),
        color_space: ViewerColorSpace::DisplayP3,
        format: SurfaceFormat::Rgba16Float,
        quality: FrameQuality {
            resolution: PreviewResolution::Half,
            representation: Representation::Proxy,
            draft_3d: false,
            cached: true,
            stand_in: false,
        },
        overlay: Some(OverlayGeometry {
            viewport: Rect {
                x: 12.5,
                y: 8.0,
                width: 1600.0,
                height: 900.0,
            },
            layers: vec![OverlayLayer {
                layer: "title".into(),
                world_2d: Some(MAT3_IDENTITY),
                world_3d: None,
                bounds: [[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [0.0, 40.0]],
                paths: vec![vec![
                    PathCommand::MoveTo { x: 0.0, y: 0.0 },
                    PathCommand::CubicTo {
                        x1: 10.0,
                        y1: 0.0,
                        x2: 20.0,
                        y2: 10.0,
                        x: 20.0,
                        y: 20.0,
                    },
                    PathCommand::LineTo { x: 0.0, y: 20.0 },
                    PathCommand::Close,
                ]],
            }],
            caret: Some(Caret {
                top: [40.0, 2.0],
                bottom: [40.0, 38.0],
            }),
        }),
        depth_id: Some(DepthIdSurface {
            pool_index: 9,
            seq: seq(11),
        }),
        view_seq: seq(17),
        duration: Some(ticks(11_760_000)),
    }
}

pub fn evaluated_scene() -> EvaluatedScene {
    let layer = |id: &str, kind: LayerKind, source: LayerSource| EvaluatedLayer {
        id: id.into(),
        kind,
        compat: Compat::Level1,
        visible: true,
        opacity: 1.0,
        blend: crate::color::BlendMode::Normal,
        transform: if kind.is_3d() {
            LayerTransform::World {
                matrix: MAT4_IDENTITY,
            }
        } else {
            LayerTransform::Flat {
                matrix: MAT3_IDENTITY,
            }
        },
        group: None,
        source,
        effects: vec![],
        masks: vec![],
        matte: None,
        transition: None,
        motion_blur: false,
    };
    let mut title = layer(
        "title",
        LayerKind::Text,
        LayerSource::Vector {
            content_key: "text:9f2c".into(),
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 120.0,
            },
        },
    );
    title.compat = Compat::Level2;
    title.blend = crate::color::BlendMode::Screen;
    title.opacity = 0.75;
    title.effects = vec![EvaluatedEffect {
        instance: "fx1".into(),
        definition: "powermove.blur".into(),
        enabled: true,
        compat: Compat::Level2,
        params: vec![
            NamedParam {
                key: "radius".into(),
                value: ParamValue::Number(12.5),
            },
            NamedParam {
                key: "tint".into(),
                value: ParamValue::Color([1.0, 0.5, 0.25]),
            },
            NamedParam {
                key: "invert".into(),
                value: ParamValue::Toggle(false),
            },
            NamedParam {
                key: "offset".into(),
                value: ParamValue::Vector(vec![0.0, 0.0]),
            },
        ],
    }];
    title.masks = vec![
        EvaluatedMask::Ellipse {
            center: [320.0, 60.0],
            size: [600.0, 100.0],
            rotation_degrees: 15.0,
            feather: 4.0,
            mode: MaskMode::Add,
            enabled: true,
        },
        EvaluatedMask::Path {
            content_key: "mask:01".into(),
            feather: 0.0,
            mode: MaskMode::Subtract,
            enabled: false,
        },
    ];
    title.matte = Some(MatteRef {
        source: "bg".into(),
        mode: MatteMode::Luma,
    });
    title.transition = Some(EvaluatedTransition {
        definition: "powermove.fade".into(),
        progress: 0.5,
        params: vec![],
    });
    EvaluatedScene {
        composition: "main".into(),
        revision: rev(42),
        time: ticks(705_600_000),
        frame_rate: FrameRate::new(30000, 1001),
        size: Size::new(1920, 1080),
        background: Background::Linear {
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: Rgba {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                },
                GradientStop {
                    position: 1.0,
                    color: Rgba {
                        r: 1.2,
                        g: 1.0,
                        b: 0.9,
                        a: 1.0,
                    },
                },
            ],
            angle_degrees: 90.0,
        },
        shutter: Shutter::from_frames(0.5),
        view: None,
        layers: vec![
            layer(
                "bg",
                LayerKind::Video,
                LayerSource::Media {
                    asset: "a1".into(),
                    source_time: ticks(-1),
                    always_original: false,
                },
            ),
            layer("cube", LayerKind::Object3d, LayerSource::World { world: 0 }),
            layer("sun", LayerKind::Light3d, LayerSource::None),
            title,
        ],
        worlds: vec![World3d {
            camera: Camera3d {
                layer: None,
                view: MAT4_IDENTITY,
                projection: Projection::Perspective {
                    fov_y_degrees: 45.0,
                    near: 0.1,
                    far: 1000.0,
                },
            },
            lights: vec![
                Light3d {
                    layer: Some("sun".into()),
                    kind: LightKind::Sun,
                    color: [1.0; 3],
                    intensity: 3.0,
                    world: MAT4_IDENTITY,
                    casts_shadow: true,
                    embedded: false,
                },
                Light3d {
                    layer: None,
                    kind: LightKind::Spot {
                        distance: 0.0,
                        angle_degrees: 30.0,
                        penumbra: 0.2,
                        decay: 2.0,
                    },
                    color: [1.0, 0.9, 0.8],
                    intensity: 10.0,
                    world: MAT4_IDENTITY,
                    casts_shadow: false,
                    embedded: true,
                },
            ],
            objects: vec![Object3d {
                layer: "cube".into(),
                model: ModelSource::Imported {
                    asset: "glb1".into(),
                    animation_time: ticks(0),
                },
                world: MAT4_IDENTITY,
                opacity: 1.0,
                writes_depth: true,
                origin_depth: 6.0,
            }],
            tone_mapping: ToneMapping3d::AcesFilmicR180 { exposure: 1.0 },
            blender: false,
        }],
        depth_sets: vec![DepthSet {
            world: 0,
            layers: vec!["cube".into()],
            shadow_casters: vec!["cube".into()],
            background_layer: "cube".into(),
        }],
        ordering: vec![OrderedRun {
            start: 1,
            order: vec!["sun".into(), "cube".into()],
        }],
        failures: vec![FailureReport {
            revision: rev(42),
            error: EngineError::new(
                ErrorCode::DefinitionMissing,
                ErrorScope::Layer {
                    layer: "title".into(),
                },
                "The effect is not installed; its settings are kept.",
            ),
        }],
    }
}

pub fn effect_snapshot() -> DefinitionSnapshot {
    let body = "o = src(v_uv) * u_amount;";
    DefinitionSnapshot {
        id: "powermove.tint".into(),
        kind: DefinitionKind::Effect,
        version: 0,
        origin: DefinitionOrigin::Fork,
        renderer: None,
        glsl: Some(WrappedGlsl {
            source: format!(
                "#version 300 es\nprecision highp float;\nuniform float u_amount;\nvoid main(){{\n{body}\n}}\n"
            ),
            body_line_offset: 4,
            body_utf16_len: body.encode_utf16().count() as u32,
            body_utf8_len: body.len() as u32,
        }),
        params: vec![
            ParamSpec::Number {
                key: "amount".into(),
                default: 1.0,
                min: 0.0,
                max: 2.0,
                step: Some(0.01),
            },
            ParamSpec::Color {
                key: "tint".into(),
                default: "#ff8800".into(),
            },
            ParamSpec::Toggle {
                key: "invert".into(),
                default: false,
            },
        ],
        flags: DefinitionFlags {
            passes: 2,
            keep_orig: true,
            backdrop: true,
            viewport_safe: true,
            viewport_padding: vec!["amount".into()],
            ..DefinitionFlags::default()
        },
    }
}

pub fn layer_snapshot() -> DefinitionSnapshot {
    DefinitionSnapshot {
        id: "powermove.3d.light".into(),
        kind: DefinitionKind::Layer,
        version: 3,
        origin: DefinitionOrigin::BuiltIn,
        renderer: Some(LayerRenderer::Layer3d {
            role: Layer3dRole::Light,
        }),
        glsl: None,
        params: vec![],
        flags: DefinitionFlags::default(),
    }
}

pub fn export_settings() -> ExportSettings {
    ExportSettings {
        format: ExportFormat::ProRes {
            profile: ProResProfile::P4444Xq,
        },
        alpha: AlphaOutput::Premultiplied,
        quality: QualityTier::Max,
        size: Size::new(3840, 2160),
        frame_rate: FrameRate::new(24000, 1001),
        range: TickRange {
            start: ticks(0),
            end: ticks(7_056_000_000),
        },
        audio: true,
        motion_blur: true,
        color: OutputColor::Rec709Broadcast,
        precision: Precision::Float32,
        dither: false,
        draft_from_proxies: false,
        delivery: DeliveryOptions {
            audio_coding: Some(AudioCoding::Pcm24),
            timecode: Some(ExportTimecode {
                start: "01:00:00:00".into(),
                drop_frame: false,
                reel: Some("A001".into()),
            }),
            hide_captions: false,
            preview: true,
            sequence_name: None,
        },
    }
}

pub fn blender_job() -> BlenderJobRequest {
    BlenderJobRequest {
        job: "blender-1".into(),
        priority: PriorityClass::UserJob,
        snapshot: SnapshotRef {
            hash: hash(1),
            encoding: SnapshotEncoding::RenderSnapshotJsonV1,
            handle: handle(1),
            bytes: bytes(65_536),
        },
        worker_script_hash: hash(2),
        assets: vec![BlenderAsset {
            asset: "tex1".into(),
            hash: hash(3),
            handle: handle(3),
        }],
        settings: BlenderSettings {
            engine: BlenderEngine::Cycles,
            samples: 128,
            preview: false,
            preview_scale: 0.5,
            denoise: true,
            device: RequestedDevice::Auto,
        },
        surfaces: vec![BlenderSurfaceRequest {
            layer: "cube".into(),
            composition_path: vec!["main".into(), "inner".into()],
            time: ticks(-705_600),
            shutter: Some(ShutterSample {
                index: 3,
                count: 16,
            }),
            size: Size::new(1920, 1080),
            key: hash(4),
        }],
        batch_layers: false,
        output: handle(5),
    }
}

/// Every request kind, in declaration order.
pub fn requests() -> Vec<Envelope> {
    let id = |n| n;
    vec![
        request(
            id(1),
            Request::OpenSession(OpenSession {
                document: "doc-1".into(),
                role: SessionRole::Interactive,
                process: EngineProcess::Interactive,
                color: project_color(),
            }),
        ),
        request(id(2), Request::CloseSession),
        request(
            id(3),
            Request::Resync(Resync {
                document: "doc-1".into(),
                revision: rev(40),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::CacheFile {
                    handle: handle(6),
                    length: bytes(123_456_789),
                },
                hash: hash(7),
            }),
        ),
        request(
            id(4),
            Request::CheckConsistency(ConsistencyCheck {
                document: "doc-1".into(),
                revision: rev(42),
                hash: hash(8),
            }),
        ),
        request(id(5), Request::RenderFrame(viewer_frame_request())).with_revision(rev(42)),
        request(
            id(6),
            Request::Evaluate(EvaluateRequest {
                composition: "main".into(),
                time: ticks(0),
                view: Some(view_override()),
            }),
        ),
        request(
            id(7),
            Request::AgentFrames(AgentFrameRequest::render_frames_default(
                "run-1".into(),
                vec![
                    AgentFrame {
                        composition: "main".into(),
                        time: ticks(0),
                    },
                    AgentFrame {
                        composition: "main".into(),
                        time: ticks(1_411_200_000),
                    },
                ],
            )),
        )
        .with_revision(rev(42)),
        request(
            id(8),
            Request::SubmitJob(SubmitJob {
                job: "job-1".into(),
                class: PriorityClass::AgentExport,
                run: Some("run-1".into()),
                spec: JobSpec::Export {
                    session: session(),
                    composition: "main".into(),
                    settings: export_settings(),
                },
                output: Some(OutputChoice::AgentAutomatic {
                    suggested_name: "Launch cut".into(),
                    extension: "mov".into(),
                }),
            }),
        ),
        request(
            id(9),
            Request::CancelJobs(CancelJobs::Run {
                run: "run-1".into(),
            }),
        ),
        request(
            id(10),
            Request::PushDefinitions(DefinitionsUpdate {
                generation: seq(5),
                full: false,
                upserts: vec![effect_snapshot(), layer_snapshot()],
                removals: vec![DefinitionRemoval {
                    id: "acme.glitch".into(),
                    kind: DefinitionKind::Transition,
                }],
            }),
        ),
        request(
            id(11),
            Request::GrantLease(Lease {
                process: EngineProcess::Job,
                epoch: seq(3),
                bytes: bytes(2 << 30),
                reason: LeaseReason::PlaybackStarted,
                ack_deadline_ms: 500,
            }),
        ),
        request(
            id(12),
            Request::RequestLease(LeaseRequest {
                process: EngineProcess::Interactive,
                wanted: bytes(6 << 30),
                reason: LeaseReason::Requested,
            }),
        ),
        request(
            id(13),
            Request::MintHandle(MintHandle {
                kind: CapabilityKind::Cache {
                    cache: CacheKind::BlenderSurfaces,
                },
                session: session(),
            }),
        ),
        request(
            id(40),
            Request::ResolveHandle(ResolveHandle {
                handle: handle(15),
                session: session(),
            }),
        ),
        request(
            id(14),
            Request::OutputDestination(OutputDestinationRequest {
                job: "job-1".into(),
                choice: OutputChoice::AgentAutomatic {
                    suggested_name: "Launch cut".into(),
                    extension: "mov".into(),
                },
                overwrite: OverwritePolicy::UniqueName,
                folder: false,
            }),
        ),
        request(
            id(15),
            Request::CommitOutput(CommitOutput {
                job: "job-1".into(),
                temporary: handle(9),
                commit: true,
                verification: Some(OutputVerification {
                    codec: "ap4x".into(),
                    frames: 240,
                    audio_samples: Count::new(480_480).expect("count"),
                    bytes: bytes(9_876_543_210),
                }),
            }),
        ),
        request(
            id(16),
            Request::HardwareSessions(HardwareSessionRequest {
                kind: HardwareSessionKind::ProResEncode,
                delta: -1,
            }),
        ),
        request(id(17), Request::BlenderRender(blender_job())),
        request(id(18), Request::BlenderStatus),
        request(
            id(19),
            Request::BlenderModel(BlenderModelJob {
                job: "model-1".into(),
                document: "doc-1".into(),
                revision: rev(41),
                operation: BlenderModelOperation::Import {
                    source: "blend1".into(),
                    object: Some("Rocket".into()),
                },
                assets: vec![BlenderAsset {
                    asset: "blend1".into(),
                    hash: hash(10),
                    handle: handle(10),
                }],
            }),
        ),
        request(
            id(20),
            Request::OpenProject(OpenProject {
                project: "p-1".into(),
                store: handle(11),
                pinned_by: Some("run-1".into()),
            }),
        ),
        request(
            id(21),
            Request::CloseProject(CloseProject {
                instance: InstanceId(1),
            }),
        ),
        request(
            id(22),
            Request::Persist(Persist {
                instance: InstanceId(1),
                reason: PersistReason::ToolCall,
            }),
        ),
        request(
            id(23),
            Request::RegisterExtension(RegisterExtension {
                instance: InstanceId(1),
                extension: "acme.glow".into(),
                version: "1.2.0".into(),
                source: ExtensionSource::AgentPublished,
                bundle: handle(12),
            }),
        ),
        request(
            id(24),
            Request::ToolCall(ToolCall {
                instance: InstanceId(1),
                run: "run-1".into(),
                call_id: "call-1".into(),
                tool: "apply_commands".into(),
                arguments: serde_json::json!({ "commands": [{ "op": "set", "path": "layers.title.opacity", "value": 0.5 }] }),
            }),
        ),
        request(
            id(26),
            Request::ProbeMedia(crate::media::ProbeMedia {
                handle: handle(13),
                interpret: Some(r#"{"primaries":"displayP3","fieldOrder":"progressive"}"#.into()),
            }),
        ),
        request(
            id(27),
            Request::ShaderFallback(crate::shader_fallback::HostShaderFallback {
                request: fallback_effect_request(),
                area: handle(14),
                textures: vec!["fallback-0.rgba16f".into(), "fallback-1.rgba16f".into()],
                output: "fallback-out.rgba16f".into(),
            }),
        ),
    ]
}

/// An effect only the fallback runner draws: its input and backdrop, two
/// passes.
fn fallback_effect_request() -> crate::shader_fallback::ShaderFallbackRequest {
    use crate::shader_fallback::{ShaderFallbackCall, ShaderFallbackRequest, ShaderFallbackUnit};
    let pass = |p: f64| {
        vec![
            ShaderFallbackCall {
                function: "1i".into(),
                name: "u_tex".into(),
                v: vec![0.0],
                alias: None,
            },
            ShaderFallbackCall {
                function: "1i".into(),
                name: "u_backdrop".into(),
                v: vec![2.0],
                alias: None,
            },
            ShaderFallbackCall {
                function: "1i".into(),
                name: "u_pass".into(),
                v: vec![p],
                alias: None,
            },
            ShaderFallbackCall {
                function: "1f".into(),
                name: "u_amount".into(),
                v: vec![0.5],
                alias: Some("u_p0".into()),
            },
        ]
    };
    ShaderFallbackRequest {
        key: "7a".repeat(32),
        layer: "layer-4".into(),
        vertex: "#version 300 es\nvoid main(){}".into(),
        fragment: "#version 300 es\nprecision highp float;\nuniform sampler2D u_tex;\nuniform highp isampler2D u_lut;\nout vec4 o;\nvoid main(){ o = texture(u_tex, vec2(0.5)); }".into(),
        width: 2,
        height: 1,
        calls: pass(0.0),
        stage: "bind".into(),
        message: "sampler `u_lut` (integer)".into(),
        textures: 2,
        units: vec![
            ShaderFallbackUnit { unit: 0, texture: 0, chain: true },
            ShaderFallbackUnit { unit: 2, texture: 1, chain: false },
        ],
        more_passes: vec![pass(1.0)],
    }
}

/// Every response kind, in declaration order.
pub fn responses() -> Vec<Envelope> {
    vec![
        response(1, Response::Ack),
        response(2, Response::SessionOpened { session: session() }),
        response(
            3,
            Response::PatchesApplied(PatchesApplied {
                document: "doc-1".into(),
                revision: rev(42),
            }),
        ),
        response(
            4,
            Response::Consistency(ConsistencyResult {
                revision: rev(42),
                matches: false,
                hash: hash(13),
            }),
        ),
        response(5, Response::Frame(frame_descriptor())),
        response(
            6,
            Response::Image(RenderedImage {
                width: 320,
                height: 180,
                revision: rev(42),
                quality: FrameQuality {
                    resolution: PreviewResolution::Full,
                    representation: Representation::Original,
                    draft_3d: true,
                    cached: false,
                    stand_in: false,
                },
                image: ImagePayload::Inline {
                    bytes: Bytes(vec![0x89, b'P', b'N', b'G']),
                },
            }),
        ),
        response(7, Response::Scene(Box::new(evaluated_scene()))),
        response(
            8,
            Response::AgentFrames(AgentFrameResult {
                outcomes: vec![
                    AgentFrameOutcome::Rendered {
                        index: 0,
                        revision: rev(42),
                        width: 720,
                        height: 405,
                        image: ImagePayload::CacheFile {
                            handle: handle(14),
                            length: bytes(48_000),
                        },
                        blender: BlenderUse::Pending,
                    },
                    AgentFrameOutcome::Failed {
                        index: 1,
                        error: EngineError::new(
                            ErrorCode::MediaOffline,
                            ErrorScope::Asset { asset: "a1".into() },
                            "Media offline · A001_C003.mov.",
                        )
                        .with_retry(Retry::AfterUserAction)
                        .with_detail(ErrorDetail::Offline {
                            assets: vec!["a1".into()],
                        }),
                    },
                ],
                queued_ms: 180,
            }),
        ),
        response(
            9,
            Response::JobAccepted(JobAccepted {
                job: "job-1".into(),
                state: JobState::Queued,
                position: 2,
            }),
        ),
        response(
            10,
            Response::JobsCancelled(JobsCancelled {
                jobs: vec!["job-1".into(), "job-2".into()],
            }),
        ),
        response(
            11,
            Response::DefinitionsApplied(DefinitionsApplied {
                generation: seq(5),
                failures: vec![
                    EngineError::new(
                        ErrorCode::ShaderCompile,
                        ErrorScope::Definition {
                            definition: "acme.glow".into(),
                        },
                        "The shader does not compile.",
                    )
                    .with_detail(ErrorDetail::ShaderCompile {
                        diagnostics: vec![ShaderDiagnostic {
                            line: 3,
                            column: Some(14),
                            message: "'u_amout' : undeclared identifier".into(),
                        }],
                    }),
                ],
            }),
        ),
        response(12, Response::LeaseUsage(lease_usage())),
        response(
            13,
            Response::Lease(Lease {
                process: EngineProcess::Interactive,
                epoch: seq(1),
                bytes: bytes(6 << 30),
                reason: LeaseReason::Initial,
                ack_deadline_ms: 1000,
            }),
        ),
        response(
            14,
            Response::Handle(HandleGrant {
                handle: handle(15),
                kind: CapabilityKind::MediaRead { asset: "a1".into() },
                content_hash: Some(hash(16)),
            }),
        ),
        response(
            40,
            Response::HandleResolved(HandleResolved {
                handle: handle(15),
                kind: CapabilityKind::MediaRead { asset: "a1".into() },
                access: HandleAccess::Read,
                path: "/Users/someone/Movies/Interview A.mov".into(),
            }),
        ),
        response(
            15,
            Response::OutputDestination(OutputDestination {
                job: "job-1".into(),
                temporary: handle(17),
            }),
        ),
        response(
            16,
            Response::OutputCommitted(OutputCommitted {
                job: "job-1".into(),
                path: Some("/Users/someone/Powermove/Launch cut 2.mov".into()),
            }),
        ),
        response(
            17,
            Response::HardwareSessions(HardwareSessionGrant {
                kind: HardwareSessionKind::Decode,
                held: 3,
            }),
        ),
        response(
            18,
            Response::BlenderStatus(BlenderStatus {
                available: true,
                version: Some("4.5.3".into()),
                error: None,
            }),
        ),
        response(
            19,
            Response::ProjectOpened(ProjectOpened {
                instance: InstanceId(1),
                document: "doc-1".into(),
                revision: rev(1),
            }),
        ),
        response(
            20,
            Response::Persisted(Persisted {
                instance: InstanceId(1),
                revision: rev(42),
                bytes: bytes(1_048_576),
            }),
        ),
        response(
            21,
            Response::ExtensionRegistered(ExtensionRegistered {
                instance: InstanceId(1),
                extension: "acme.glow".into(),
                definitions_generation: seq(6),
            }),
        ),
        response(
            22,
            Response::ToolResult(ToolResult {
                instance: InstanceId(1),
                call_id: "call-1".into(),
                revision: rev(43),
                content: serde_json::json!([{ "type": "text", "text": "Applied 1 command." }]),
                is_error: false,
            }),
        ),
        response(
            24,
            Response::MediaFacts(Box::new(crate::media::MediaFacts {
                kind: crate::media::MediaKind::Video,
                facts: vec![crate::media::FactLine {
                    label: "Codec".into(),
                    value: "ProRes 422 HQ".into(),
                }],
                settings: vec![
                    crate::media::InterpretSetting {
                        member: "primaries".into(),
                        value: "displayP3".into(),
                        automatic: "bt709".into(),
                        origin: crate::media::FactOrigin::Override,
                    },
                    crate::media::InterpretSetting {
                        member: "range".into(),
                        value: "limited".into(),
                        automatic: "limited".into(),
                        origin: crate::media::FactOrigin::Default {
                            rule: "Untagged Y′CbCr video is limited range".into(),
                        },
                    },
                    crate::media::InterpretSetting {
                        member: "fieldOrder".into(),
                        value: "progressive".into(),
                        automatic: "topFirst".into(),
                        origin: crate::media::FactOrigin::File {
                            source: "fiel".into(),
                        },
                    },
                ],
                issues: vec![],
                display_size: Some([1920, 1080]),
                duration: Some(12.5),
            })),
        ),
    ]
}

fn lease_usage() -> LeaseUsage {
    let categories = LeaseCategories {
        frame_cache: bytes(1 << 30),
        render_cache: bytes(512 << 20),
        blender_surfaces: bytes(0),
        geometry_3d: bytes(64 << 20),
        surface_pool: bytes(256 << 20),
        decoders: bytes(128 << 20),
        pipelines: bytes(32 << 20),
        other: bytes(16 << 20),
    };
    LeaseUsage {
        process: EngineProcess::Job,
        epoch: seq(3),
        total: categories.total().expect("sum"),
        categories,
        ack: true,
    }
}

/// Every event kind, in declaration order.
pub fn events() -> Vec<Envelope> {
    vec![
        event(Event::ApplyPatches(PatchBatch {
            document: "doc-1".into(),
            base_revision: rev(41),
            revision: rev(42),
            origin: PatchOrigin::Edit,
            encoding: PatchEncoding::CanonicalV1,
            patch_count: 2,
            body: Bytes(vec![0x92, 0x01, 0x02]),
        })),
        event(Event::PatchesApplied(PatchesApplied {
            document: "doc-1".into(),
            revision: rev(42),
        })),
        event(Event::Frame(frame_descriptor())),
        event(Event::ReleaseFrame(FrameRelease {
            pool_index: 3,
            seq: seq(MAX_SAFE_INTEGER as u64),
        })),
        event(Event::SurfacePool(SurfacePool {
            generation: seq(2),
            surfaces: (0..5)
                .map(|i| SurfaceDescriptor {
                    pool_index: i,
                    size: Size::new(3840, 2160),
                    format: SurfaceFormat::Rgba16Float,
                    platform: PlatformSurface::IoSurface {
                        bridge_index: 100 + i,
                    },
                })
                .collect(),
        })),
        event(Event::Viewer(ViewerState {
            composition: "main".into(),
            size: Size::new(3024, 1964),
            device_pixel_ratio: 2.0,
            zoom: 0.5,
            pan: [-12.0, 4.5],
            resolution: PreviewResolution::Automatic,
            media: MediaPreference::Proxies,
            color_space: ViewerColorSpace::DisplayP3,
            view: Some(view_override()),
            play_every_frame: true,
            edr: true,
            view_seq: seq(17),
            vsync: Some(VsyncHint {
                host_time: HostMicros::new(123_456_700_000).expect("micros"),
                period_us: 8_333.333_333_333_334,
            }),
        })),
        event(Event::Transport(TransportCommand::Seek {
            time: ticks(-705_600_000),
            mode: SeekMode::Scrub,
        })),
        event(Event::Audio(AudioMessage::Levels(AudioLevels {
            media_time: ticks(1_234_567_890),
            master: ChannelLevels {
                peak: vec![0.5, 0.25],
                rms: vec![0.125, 0.0625],
            },
            strips: vec![StripLevels {
                id: "layer-1".into(),
                levels: ChannelLevels {
                    peak: vec![0.5, 0.25],
                    rms: vec![0.125, 0.0625],
                },
            }],
        }))),
        event(Event::Audio(AudioMessage::Monitor {
            solo: vec!["layer-1".into()],
        })),
        event(Event::ShaderFallback(
            crate::shader_fallback::ShaderFallbackMessage::Requests {
                requests: vec![crate::shader_fallback::ShaderFallbackRequest {
                    key: "5e".repeat(32),
                    layer: "layer-3".into(),
                    vertex: "#version 300 es\nvoid main(){}".into(),
                    fragment: "#version 300 es\nprecision highp float;\nuniform samplerCube u;\nout vec4 fragColor;\nvoid main(){ fragColor = texture(u, vec3(1.0)); }".into(),
                    width: 2,
                    height: 1,
                    calls: vec![crate::shader_fallback::ShaderFallbackCall {
                        function: "2f".into(),
                        name: "iResolution".into(),
                        v: vec![2.0, 1.0],
                        alias: None,
                    }],
                    stage: "bind".into(),
                    message: "sampler `u` (Cube)".into(),
                    textures: 0,
                    units: vec![],
                    more_passes: vec![],
                }],
            },
        )),
        event(Event::ShaderFallback(
            crate::shader_fallback::ShaderFallbackMessage::Input(
                crate::shader_fallback::ShaderFallbackInput {
                    key: "7a".repeat(32),
                    texture: 1,
                    width: 2,
                    height: 1,
                    row: 0,
                    rows: 1,
                    rgba16f: crate::common::Bytes(vec![0, 0, 0, 0, 0, 0, 0, 0x3c, 0, 0x3c, 0, 0, 0, 0, 0, 0x3c]),
                },
            ),
        )),
        event(Event::ShaderFallback(
            crate::shader_fallback::ShaderFallbackMessage::Pixels(
                crate::shader_fallback::ShaderFallbackPixels {
                    key: "5e".repeat(32),
                    width: 2,
                    height: 1,
                    row: 0,
                    rows: 1,
                    rgba16f: crate::common::Bytes(vec![0, 0, 0, 0, 0, 0, 0, 0x3c, 0, 0, 0, 0, 0, 0, 0, 0x3c]),
                },
            ),
        )),
        event(Event::ShaderFallback(
            crate::shader_fallback::ShaderFallbackMessage::Failed {
                key: "5e".repeat(32),
                reason: "compile".into(),
                message: "the program does not link".into(),
            },
        )),
        event(Event::Audio(AudioMessage::PeaksWanted {
            asset: "asset-7".into(),
        })),
        event(Event::Audio(AudioMessage::Peaks(AudioPeaks {
            asset: "asset-7".into(),
            peaks_per_second: 100.0,
            duration: 0.03,
            peaks: vec![0.0, 0.5, 0.25],
            error: None,
        }))),
        event(Event::Clock(ClockSample {
            seq: seq(99),
            epoch: 4,
            state: TransportState::Playing,
            host_time: HostMicros::new(123_456_789_012).expect("micros"),
            media_time: ticks(1_234_567_890),
            rate: PlaybackRate { num: -2, den: 1 },
            output_latency_us: 11_610,
            sample_rate: 48_000,
            loop_range: Some(TickRange {
                start: ticks(0),
                end: ticks(7_056_000_000),
            }),
        })),
        event(Event::Deadline(DeadlineSignal::DeadlineMiss {
            at: HostMicros::new(123_456_800_000).expect("micros"),
            frame_seq: seq(812),
            overrun_us: 900,
        })),
        event(Event::Deadline(DeadlineSignal::FrameDone {
            at: HostMicros::new(123_456_810_500).expect("micros"),
            frame_seq: seq(813),
            tick: HostMicros::new(123_456_800_000).expect("micros"),
            next_submit: HostMicros::new(123_456_816_666).expect("micros"),
            late: false,
        })),
        event(Event::Slicing(SlicingPolicy {
            blender: BlenderThrottle::Throttle { max_workers: 1 },
            ..SlicingPolicy::DEFAULT
        })),
        event(Event::Failure(FailureReport {
            revision: rev(42),
            error: EngineError::new(
                ErrorCode::WatchdogTimeout,
                ErrorScope::Effect {
                    layer: "title".into(),
                    effect: "fx1".into(),
                    definition: "acme.glow".into(),
                },
                "Glow took too long on the GPU and was turned off.",
            ),
        })),
        event(Event::Crash(CrashReport {
            process: EngineProcess::Interactive,
            attempt: 2,
            exit: "SIGSEGV".into(),
            suspects: vec![ErrorScope::Asset { asset: "a1".into() }],
            marked_failed: vec![ErrorScope::Asset { asset: "a1".into() }],
            restart_in_ms: 500,
        })),
        event(Event::InFlight(InFlight {
            suspects: vec![
                ErrorScope::Asset { asset: "a1".into() },
                ErrorScope::Effect {
                    layer: "title".into(),
                    effect: "fx1".into(),
                    definition: "acme.glow".into(),
                },
            ],
        })),
        event(Event::LeaseUsage(LeaseUsage {
            ack: false,
            ..lease_usage()
        })),
        event(Event::JobStatus(JobStatus {
            job: "job-1".into(),
            class: PriorityClass::AgentExport,
            state: JobState::Running,
            progress: 0.25,
            path: None,
            error: None,
            preview: Some(JobPreview {
                frame: 24,
                width: 640,
                height: 360,
                jpeg: Bytes(vec![0xFF, 0xD8, 0xFF, 0xD9]),
            }),
        })),
        event(Event::BlenderProgress(BlenderProgress {
            job: "blender-1".into(),
            completed: 3,
            total: 16,
        })),
        event(Event::BlenderSurface(BlenderSurface {
            job: "blender-1".into(),
            request_index: 0,
            key: hash(4),
            size: Size::new(1920, 1080),
            crop: PixelRect {
                x: 400,
                y: 200,
                width: 800,
                height: 600,
            },
            handle: handle(18),
            bytes: bytes(3_840_000),
            device: ResolvedDevice::Metal {
                name: "Apple M4 Max".into(),
            },
        })),
        event(Event::Fact(EngineFact::RelinkProposal {
            asset: "a1".into(),
            candidate: handle(19),
            reason: RelinkReason::Timecode,
        })),
        event(Event::Fact(EngineFact::BlenderResult {
            job: "model-1".into(),
            revision: rev(41),
            files: vec![BlenderOutputFile {
                name: "Rocket.glb".into(),
                hash: hash(20),
                handle: handle(20),
                bytes: bytes(2_000_000),
            }],
            manifest: "{\"objects\":[\"Rocket\"]}".into(),
        })),
    ]
}

/// The handshake, a cancel, and a connection-level error.
pub fn control() -> Vec<Envelope> {
    let hello = Hello {
        role: PeerRole::Renderer,
        wire: WIRE_VERSION,
        build: "0123abcd".into(),
        contracts: local_ranges(),
        max_message_bytes: 16 << 20,
    };
    let welcome = Welcome {
        role: PeerRole::InteractiveEngine,
        wire: WIRE_VERSION,
        build: "0123abcd".into(),
        contracts: local_ranges()
            .iter()
            .map(|r| ContractVersion {
                contract: r.contract,
                version: r.max,
            })
            .collect(),
        max_message_bytes: 16 << 20,
    };
    vec![
        Envelope::new(Message::Hello(hello)),
        Envelope::new(Message::Welcome(welcome)),
        Envelope::new(Message::Cancel {
            id: RequestId::new(7).expect("id"),
        }),
        Envelope::new(Message::Error {
            id: RequestId::new(5).expect("id"),
            error: EngineError::new(
                ErrorCode::StaleRevision,
                ErrorScope::Request {
                    id: RequestId::new(5).expect("id"),
                },
                "The engine has not caught up with this edit yet.",
            )
            .with_retry(Retry::AfterResync)
            .with_detail(ErrorDetail::StaleRevision {
                applied: rev(40),
                wanted: rev(42),
            }),
        }),
    ]
}

/// Every sample, named for its fixture file.
pub fn all() -> Vec<(String, Envelope)> {
    let mut named = Vec::new();
    for envelope in control() {
        let name = match &envelope.message {
            Message::Hello(_) => "hello".to_owned(),
            Message::Welcome(_) => "welcome".to_owned(),
            Message::Cancel { .. } => "cancel".to_owned(),
            Message::Error { .. } => "error".to_owned(),
            _ => unreachable!("control samples"),
        };
        named.push((name, envelope));
    }
    for (prefix, list) in [
        ("request", requests()),
        ("response", responses()),
        ("event", events()),
    ] {
        for envelope in list {
            let tag = match &envelope.message {
                Message::Request { body, .. } => tag_of(body),
                Message::Response { body, .. } => tag_of(body),
                Message::Event { body } => tag_of(body),
                _ => unreachable!("typed samples"),
            };
            let mut name = format!("{prefix}-{tag}");
            // Facts, deadline signals, and audio messages have several samples each, named by their inner tag.
            let inner = match &envelope.message {
                Message::Event {
                    body: Event::Fact(fact),
                } => serde_json::to_value(fact)
                    .ok()
                    .and_then(|v| v["kind"].as_str().map(str::to_owned)),
                Message::Event {
                    body: Event::Deadline(signal),
                } => serde_json::to_value(signal)
                    .ok()
                    .and_then(|v| v["signal"].as_str().map(str::to_owned)),
                Message::Event {
                    body: Event::Audio(message),
                } => serde_json::to_value(message)
                    .ok()
                    .and_then(|v| v["audio"].as_str().map(str::to_owned)),
                Message::Event {
                    body: Event::ShaderFallback(message),
                } => serde_json::to_value(message)
                    .ok()
                    .and_then(|v| v["fallback"].as_str().map(str::to_owned)),
                _ => None,
            };
            if let Some(inner) = inner {
                name.push('-');
                name.push_str(&inner);
            }
            named.push((name, envelope));
        }
    }
    named
}

fn tag_of<T: serde::Serialize>(body: &T) -> String {
    serde_json::to_value(body)
        .ok()
        .and_then(|v| v["type"].as_str().map(str::to_owned))
        .expect("tagged body")
}

/// Exhaustive index of request variants; add a sample when this grows.
pub const fn request_index(request: &Request) -> usize {
    match request {
        Request::OpenSession(_) => 0,
        Request::CloseSession => 1,
        Request::Resync(_) => 2,
        Request::CheckConsistency(_) => 3,
        Request::RenderFrame(_) => 4,
        Request::Evaluate(_) => 5,
        Request::AgentFrames(_) => 6,
        Request::SubmitJob(_) => 7,
        Request::CancelJobs(_) => 8,
        Request::PushDefinitions(_) => 9,
        Request::GrantLease(_) => 10,
        Request::RequestLease(_) => 11,
        Request::MintHandle(_) => 12,
        Request::OutputDestination(_) => 13,
        Request::CommitOutput(_) => 14,
        Request::HardwareSessions(_) => 15,
        Request::BlenderRender(_) => 16,
        Request::BlenderStatus => 17,
        Request::BlenderModel(_) => 18,
        Request::OpenProject(_) => 19,
        Request::CloseProject(_) => 20,
        Request::Persist(_) => 21,
        Request::RegisterExtension(_) => 22,
        Request::ToolCall(_) => 23,
        Request::ResolveHandle(_) => 24,
        Request::ProbeMedia(_) => 25,
        Request::ShaderFallback(_) => 26,
    }
}
pub const REQUEST_VARIANTS: usize = 27;

/// Exhaustive index of response variants.
pub const fn response_index(response: &Response) -> usize {
    match response {
        Response::Ack => 0,
        Response::SessionOpened { .. } => 1,
        Response::PatchesApplied(_) => 2,
        Response::Consistency(_) => 3,
        Response::Frame(_) => 4,
        Response::Image(_) => 5,
        Response::Scene(_) => 6,
        Response::AgentFrames(_) => 7,
        Response::JobAccepted(_) => 8,
        Response::JobsCancelled(_) => 9,
        Response::DefinitionsApplied(_) => 10,
        Response::LeaseUsage(_) => 11,
        Response::Lease(_) => 12,
        Response::Handle(_) => 13,
        Response::OutputDestination(_) => 14,
        Response::OutputCommitted(_) => 15,
        Response::HardwareSessions(_) => 16,
        Response::BlenderStatus(_) => 17,
        Response::ProjectOpened(_) => 18,
        Response::Persisted(_) => 19,
        Response::ExtensionRegistered(_) => 20,
        Response::ToolResult(_) => 21,
        Response::HandleResolved(_) => 22,
        Response::MediaFacts(_) => 23,
    }
}
pub const RESPONSE_VARIANTS: usize = 24;

/// Exhaustive index of event variants.
pub const fn event_index(event: &Event) -> usize {
    match event {
        Event::ApplyPatches(_) => 0,
        Event::PatchesApplied(_) => 1,
        Event::Frame(_) => 2,
        Event::ReleaseFrame(_) => 3,
        Event::SurfacePool(_) => 4,
        Event::Viewer(_) => 5,
        Event::Transport(_) => 6,
        Event::Clock(_) => 7,
        Event::Deadline(_) => 8,
        Event::Slicing(_) => 9,
        Event::Failure(_) => 10,
        Event::Crash(_) => 11,
        Event::LeaseUsage(_) => 12,
        Event::JobStatus(_) => 13,
        Event::BlenderProgress(_) => 14,
        Event::BlenderSurface(_) => 15,
        Event::Fact(_) => 16,
        Event::InFlight(_) => 17,
        Event::Audio(_) => 18,
        Event::ShaderFallback(_) => 19,
    }
}
pub const EVENT_VARIANTS: usize = 20;

/// A sample color description for a 10-bit 4:2:2 ProRes source.
pub fn prores_422_color() -> ColorDescription {
    ColorDescription {
        primaries: Primaries::Bt709,
        transfer: Transfer::DisplayE,
        matrix: MatrixCoefficients::Bt709,
        range: Range::Limited,
        subsampling: Subsampling::S422,
        siting: ChromaSiting::Left,
        alpha: AlphaMode::Opaque,
        bit_depth: 10,
    }
}

/// The golden wire fixtures directory, relative to this crate's manifest.
pub const FIXTURE_DIR_FROM_MANIFEST: &str = "tests/fixtures/wire";

/// The fixtures directory on disk.
pub fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR_FROM_MANIFEST)
}

/// Every sample encoded with the production codec, keyed by file name.
pub fn fixture_files()
-> Result<std::collections::BTreeMap<String, Vec<u8>>, crate::codec::CodecError> {
    all()
        .into_iter()
        .map(|(name, envelope)| Ok((format!("{name}.msgpack"), crate::codec::encode(&envelope)?)))
        .collect()
}
