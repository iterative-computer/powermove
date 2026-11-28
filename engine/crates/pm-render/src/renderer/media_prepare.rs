//! Plan-aware video startup before the first requested frame. The plan is
//! built with the same document, time and request as rendering, then walked
//! without submitting GPU work. Source and selected decoded frame preparation
//! run asynchronously; the presenter never waits for a hash, index or decoder.

use super::*;
use crate::composite;
use crate::plan::{Content, Source};

/// Result of preparing media for a planned frame. A source that cannot be
/// prepared is reported here and will still get its usual render failure.
#[derive(Debug, Default)]
pub struct MediaPreparation {
    pub queued_videos: usize,
    pub failures: Vec<String>,
    /// The video draws the plan selected: (layer, instance).
    pub videos: Vec<(String, u64)>,
}

fn visit_content<'a>(
    content: &'a Content,
    layer: &str,
    visit: &mut impl FnMut(&str, &'a Source),
    effects: &mut impl FnMut(&'a composite::effects::PreparedEffects),
) {
    match content {
        Content::Texture { source, .. } => {
            visit(layer, source);
            if let Source::Comp(plan) | Source::Accumulate(plan) = source {
                visit_plan(plan, visit, effects);
            }
        }
        Content::Placed(placed) => visit_content(&placed.content, layer, visit, effects),
        Content::Composite(node) => {
            if let composite::Node::Layer(layer_node) = &**node {
                for draw in &layer_node.content {
                    visit_content(&draw.content, &draw.layer, visit, effects);
                }
                if let Some(matte) = &layer_node.matte {
                    visit_plan(&matte.source, visit, effects);
                }
                if let Some(layer_effects) = &layer_node.effects {
                    effects(layer_effects);
                }
            } else if let composite::Node::Adjustment(adjustment) = &**node {
                effects(&adjustment.effects);
            }
        }
        Content::Solid(_) => {}
    }
}

fn visit_plan<'a>(
    plan: &'a CompPlan,
    visit: &mut impl FnMut(&str, &'a Source),
    effects: &mut impl FnMut(&'a composite::effects::PreparedEffects),
) {
    for draw in &plan.draws {
        visit_content(&draw.content, &draw.layer, visit, effects);
    }
}

/// Vectors the plan can resolve more than once. A cutoff must keep these
/// even if one of their draws already ran. Include nested effect inputs.
pub(crate) fn repeated_vectors(plan: &CompPlan) -> std::collections::HashSet<[u8; 32]> {
    use std::collections::HashSet;

    fn collect(plan: &CompPlan, seen: &mut HashSet<[u8; 32]>, repeated: &mut HashSet<[u8; 32]>) {
        let mut chains = Vec::new();
        visit_plan(
            plan,
            &mut |_, source| {
                if let Source::Vector(vector) = source {
                    let key = vector.key();
                    if !seen.insert(key) {
                        repeated.insert(key);
                    }
                }
            },
            &mut |effects| chains.push(effects),
        );
        for chain in chains {
            for input in chain.passes.input_plans() {
                collect(&input.plan, seen, repeated);
            }
            // An effect asset can reuse an earlier draw's raster.
            repeated.extend(chain.passes.vector_assets().iter().filter_map(|asset| {
                asset.vector.as_ref().map(crate::vector::VectorSource::key)
            }));
        }
    }

    let mut repeated = HashSet::new();
    collect(plan, &mut HashSet::new(), &mut repeated);
    repeated
}

/// The still draws of a plan, nested compositions included: each file, its
/// asset's interpretation, and the edge its draw reads it at
/// (`crate::exec::source_edge`, from the same matrix, UV window and plane
/// the draw resolves with).
type Interpret<'a> = Option<&'a pm_media::interpret::Interpretation>;

fn visit_stills(plan: &CompPlan, visit: &mut impl FnMut(&std::path::Path, Interpret, Option<u32>)) {
    fn content(
        c: &Content,
        matrix: &[f64; 6],
        plane: Option<&crate::space25d::Plane>,
        visit: &mut impl FnMut(&std::path::Path, Interpret, Option<u32>),
    ) {
        match c {
            Content::Texture { source, uv, .. } => match source {
                Source::Image {
                    path, interpret, ..
                } => visit(
                    path,
                    interpret.as_ref(),
                    crate::exec::source_edge(matrix, *uv, plane),
                ),
                Source::Comp(plan) | Source::Accumulate(plan) => visit_stills(plan, visit),
                _ => {}
            },
            Content::Placed(placed) => content(
                &placed.content,
                matrix,
                placed.plane.as_ref(),
                visit,
            ),
            Content::Composite(node) => {
                if let composite::Node::Layer(layer) = &**node {
                    for draw in &layer.content {
                        content(&draw.content, &draw.matrix, None, visit);
                    }
                    if let Some(matte) = &layer.matte {
                        visit_stills(&matte.source, visit);
                    }
                }
            }
            Content::Solid(_) => {}
        }
    }
    for draw in &plan.draws {
        content(&draw.content, &draw.matrix, None, visit);
    }
}

/// The video draws of a plan, nested compositions included.
/// Whether `held` says yes for a still `plan` draws.
pub(super) fn any_still(plan: &CompPlan, held: &mut impl FnMut(&std::path::Path) -> bool) -> bool {
    let mut any = false;
    visit_stills(plan, &mut |path, _, _| any = any || held(path));
    any
}

pub(super) fn planned_videos(plan: &CompPlan) -> Vec<crate::media::PlannedVideoDraw> {
    let mut out = Vec::new();
    visit_plan(
        plan,
        &mut |layer, source| {
            if let Source::Video {
                path,
                time,
                interpret,
                composition_rate,
                ..
            } = source
            {
                out.push(crate::media::PlannedVideoDraw {
                    layer: layer.to_owned(),
                    path: path.clone(),
                    instance: crate::exec::instance_of(layer),
                    seconds: *time,
                    interpret: interpret.clone(),
                    composition: *composition_rate,
                });
            }
        },
        &mut |_| {},
    );
    out
}

/// Scan only layers whose spans contain the requested time. A deadline also
/// bounds work when many active precomps instance the same set of layers.
fn visit_upcoming<'d>(
    comp: &Comp<'d>,
    seconds: f64,
    scale: (f64, f64),
    depth: u32,
    deadline: std::time::Instant,
    visit: &mut impl FnMut(&Comp<'d>, &'d pm_model::Node, &'d str, f64, (f64, f64)),
) {
    use pm_eval::node::{member, string, to_number};
    for (index, layer) in comp.layers().iter().enumerate() {
        if index % 64 == 0 && std::time::Instant::now() >= deadline {
            return;
        }
        let layer_node = Some(layer);
        let Some(kind) = string(member(layer_node, "type")).and_then(|s| s.as_str()) else {
            continue;
        };
        let media = matches!(kind, "image" | "video" | "precomp");
        if !media && kind != "shader" && !has_program_stages(layer) {
            continue;
        }
        let from = to_number(member(layer_node, "from"));
        let end = from + to_number(member(layer_node, "dur"));
        if (kind != "group" && !(from <= seconds && seconds < end))
            || !upcoming_value(member(layer_node, "on"), seconds).truthy()
        {
            continue;
        }
        visit(comp, layer, kind, seconds, scale);
        if kind == "precomp"
            && depth < plan::PRECOMP_MAX_DEPTH
            && let Some((id, _)) = capability::precomp_target(comp.root(), layer)
            && let Ok(child) = Comp::in_root(comp.root(), &id, comp.revision())
            && let Some(at) = upcoming_source_time(layer, seconds)
        {
            let at = crate::retime::keep_frame_rate(layer, &child, at);
            // Use the original still for nested draws, where the child's
            // projected density can differ from the outer render scale.
            visit_upcoming(&child, at, (0., 0.), depth + 1, deadline, visit);
        }
    }
}

fn has_program_stages(layer: &pm_model::Node) -> bool {
    use pm_eval::node::{array, is_str, truthy};
    array(layer.get("fx")).is_some_and(|fx| !fx.is_empty())
        || truthy(layer.get("transitionIn"))
        || truthy(layer.get("transitionOut"))
        || is_str(layer.get("type"), "extension")
}

fn upcoming_programs<'a>(
    effects: &crate::effects::Effects,
    saved: &mut crate::effects::prewarm::SavedEffects<'a>,
    layer: &'a pm_model::Node,
    seconds: f64,
    keys: &mut Vec<crate::effects::prewarm::Key>,
) {
    for fx in pm_eval::node::array(layer.get("fx")).unwrap_or_default() {
        let on = upcoming_value(fx.get("on"), seconds);
        if !matches!(on, pm_eval::JsValue::Bool(false)) {
            saved.push(effects, fx, keys);
        }
    }
    crate::effects::prewarm::push_other_programs(layer, keys);
}

/// Resolve stored values and keyframes without evaluating expressions.
fn upcoming_value(field: Option<&pm_model::Node>, time: f64) -> pm_eval::JsValue {
    use pm_eval::node::{array, is_property, js_value_opt, member};
    if !is_property(field) {
        return js_value_opt(field);
    }
    let keys = member(field, "kf");
    if array(keys).is_some_and(|keys| !keys.is_empty() && keys.len() <= 128)
        && let Ok(value) = pm_eval::keyframes::evaluate(keys, time, false, &mut Default::default())
        && !matches!(value, pm_eval::JsValue::Null | pm_eval::JsValue::Undefined)
    {
        return value;
    }
    js_value_opt(member(field, "v"))
}

fn upcoming_source_time(layer: &pm_model::Node, time: f64) -> Option<f64> {
    use pm_eval::node::{array, is_property, member, to_number, truthy};
    let data = member(Some(layer), "d");
    let source = member(data, "sourceTime");
    if upcoming_value(member(data, "timeRemap"), time).truthy() && is_property(source) {
        return (!truthy(member(source, "expr"))).then(|| {
            let seconds = upcoming_value(source, time).to_number();
            if seconds.is_nan() { 0. } else { seconds }
        });
    }
    let speed = member(data, "speed");
    // Integrating variable speed from the layer start can take longer than
    // the available slack. The foreground renderer handles these layers.
    if truthy(member(speed, "expr"))
        || array(member(speed, "kf")).is_some_and(|keys| !keys.is_empty())
    {
        return None;
    }
    let number = |key, default| {
        let value = upcoming_value(member(data, key), time);
        if matches!(value, pm_eval::JsValue::Null | pm_eval::JsValue::Undefined) {
            default
        } else {
            value.to_number()
        }
    };
    Some((time - to_number(member(Some(layer), "from"))) * number("speed", 1.) + number("trim", 0.))
}

/// Order the active composition and its instances by their approximate
/// first root-timeline use. Lookahead handles moving and expression-driven
/// retimes, and unreferenced compositions are queued last.
pub(super) fn program_layers<'a>(
    document: &'a Document,
    composition: &str,
) -> Vec<&'a pm_model::Node> {
    use pm_eval::node;
    type Ordered<'a> = (f64, &'a pm_model::Node);
    fn collect<'a>(
        comp: &Comp<'a>,
        parent_time: &dyn Fn(f64) -> f64,
        depth: u32,
        layers: &mut Vec<Ordered<'a>>,
        seen: &mut std::collections::HashSet<pm_model::JsString>,
    ) {
        seen.insert(comp.id().clone());
        for layer in comp.layers() {
            if layers.len() >= 65_536 {
                break;
            }
            if matches!(
                layer.get("on").map(pm_model::Node::kind),
                Some(pm_model::NodeKind::Bool(false))
            ) {
                continue;
            }
            let from = node::number(layer.get("from")).unwrap_or(0.);
            let end = from + node::number(layer.get("dur")).unwrap_or(0.).max(0.);
            layers.push((parent_time(from), layer));
            if depth < plan::PRECOMP_MAX_DEPTH
                && let Some((id, _)) = capability::precomp_target(comp.root(), layer)
                && let Ok(child) = Comp::in_root(comp.root(), &id, comp.revision())
            {
                let at = upcoming_source_time(layer, from).unwrap_or(0.);
                let speed = upcoming_source_time(layer, from + 1.).map_or(0., |next| next - at);
                collect(
                    &child,
                    &|time| {
                        let mapped = if speed > 0. {
                            from + (time - at) / speed
                        } else {
                            from
                        };
                        parent_time(mapped.max(from).min(end))
                    },
                    depth + 1,
                    layers,
                    seen,
                );
            }
        }
    }

    let Some(root) = document.root() else {
        return Vec::new();
    };
    let mut layers = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Ok(comp) = Comp::resolve(document, composition) {
        collect(&comp, &|time| time, 0, &mut layers, &mut seen);
    }
    layers.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut result: Vec<_> = layers.into_iter().map(|(_, layer)| layer).collect();
    if node::string(root.get("id")).is_some_and(|id| !seen.contains(id)) {
        result.extend(node::array(root.get("layers")).unwrap_or_default());
    }
    if let Some(comps) = root.get("comps") {
        for (id, comp) in node::entries(comps) {
            if !seen.contains(&id) {
                result.extend(node::array(node::member(comp.node(), "layers")).unwrap_or_default());
            }
        }
    }
    let mut visited = std::collections::HashSet::new();
    result.retain(|layer| visited.insert(std::ptr::from_ref(*layer)));
    result
}

fn upcoming_edge(
    layer: &pm_model::Node,
    record: &pm_model::Node,
    seconds: f64,
    scale: (f64, f64),
) -> Option<u32> {
    use pm_eval::node::{member, to_number, truthy};
    let layer = Some(layer);
    if scale.0 == 0. || truthy(member(layer, "parent")) || truthy(member(layer, "is3D")) {
        return None;
    }
    let data = member(layer, "d");
    let p = member(layer, "p");
    let dimension = |size, transform| {
        let native = to_number(member(Some(record), size));
        let size = upcoming_value(member(data, size), seconds).to_number();
        let channel = member(p, transform);
        if truthy(member(channel, "expr")) {
            return None;
        }
        let zoom = upcoming_value(channel, seconds).to_number();
        Some(
            if size.is_finite() && size > 0. {
                size
            } else {
                native
            } * if zoom.is_finite() {
                zoom.abs() / 100.
            } else {
                1.
            },
        )
    };
    // Use the larger output scale so rotated layers get enough pixels.
    let density = scale.0.max(scale.1);
    let (w, h) = (dimension("w", "scale.x")?, dimension("h", "scale.y")?);
    crate::exec::source_edge(
        &[w * density, 0., 0., h * density, 0., 0.],
        [0., 0., 1., 1.],
        None,
    )
}

impl Renderer {
    /// Queue preparation of the videos selected by one planned view. The
    /// worker retains each completed source and matching idle decoder until
    /// the renderer adopts them. It also offers the selected decoded frame to
    /// the shared lease-bound cache. A first render can proceed cold if the
    /// worker is unfinished or the frame does not fit that cache.
    pub fn prepare_media(
        &mut self,
        document: &Document,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        assets: &dyn AssetResolver,
    ) -> Result<MediaPreparation, RenderError> {
        request.validate()?;
        let root = document
            .root()
            .ok_or_else(|| RenderError::Composition(composition.to_owned()))?;
        project_settings(root)?;
        let comp = Comp::resolve(document, composition)
            .map_err(|_| RenderError::Composition(composition.to_owned()))?;
        let open = capability::open_composition_id(root);
        let project = open
            .as_deref()
            .and_then(|id| Comp::resolve(document, id).ok())
            .unwrap_or_else(|| comp.clone());
        let seconds = comp.seconds_at(time.get());
        let (w, h) = (request.output_size.width, request.output_size.height);
        let scale = request
            .scale
            .unwrap_or_else(|| plan::derived_scale(&comp, w, h));
        let options3d = self.options3d.for_request(request);
        self.open_channel_memo(document);
        let (plan, _) = plan::build_reusing(
            &mut self.evaluator,
            &comp,
            &project,
            seconds,
            w,
            h,
            scale,
            assets,
            effect_host(&self.effect_host, &self.effects),
            options3d,
            Some(&mut self.text),
            build_options(
                &comp,
                request,
                self.shaders.fallback_runner,
                self.isolate_layers,
            ),
            None,
            None,
        );
        self.evaluator.close_channel_memo();
        // A layer the renderer cannot draw is reported by the frame itself;
        // the media of every other layer is still prepared (refusing here
        // left a project with one such layer without any look-ahead).
        let mut result = MediaPreparation::default();
        if !request.export {
            visit_stills(&plan, &mut |path, interpret, edge| {
                self.media.prepare_image(path, edge, interpret);
            });
        }
        for v in planned_videos(&plan) {
            result.videos.push((v.layer.clone(), v.instance));
            match self.media.prepare_video(
                &v.path,
                v.instance,
                v.seconds,
                request.export,
                request.read_ahead,
                v.interpret.as_ref(),
                v.composition,
            ) {
                Ok(true) => result.queued_videos += 1,
                Ok(false) => {}
                Err(error) => {
                    result
                        .failures
                        .push(format!("{} ({}): {error:?}", v.layer, v.path.display()))
                }
            }
        }
        Ok(result)
    }

    /// Prepare media at one upcoming time on a large playback composition.
    /// All file decoding remains on the existing preparation workers.
    pub fn prepare_media_ahead(
        &mut self,
        document: &Document,
        composition: &str,
        time: Ticks,
        request: &RenderRequest,
        assets: &dyn AssetResolver,
    ) -> Result<MediaPreparation, RenderError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2);
        let comp = Comp::resolve(document, composition)
            .map_err(|_| RenderError::Composition(composition.to_owned()))?;
        let scale = request.scale.unwrap_or_else(|| {
            plan::derived_scale(&comp, request.output_size.width, request.output_size.height)
        });
        let mut result = MediaPreparation::default();
        let mut stills = std::collections::HashSet::new();
        let mut programs = Vec::new();
        let mut shaders = Vec::new();
        let mut saved = crate::effects::prewarm::SavedEffects::default();
        let native_effects = self.effect_host.is_none();
        visit_upcoming(
            &comp,
            comp.seconds_at(time.get()),
            scale,
            0,
            deadline,
            &mut |comp, layer, kind, seconds, scale| {
                use pm_eval::node::{is_str, member, string};
                if std::time::Instant::now() >= deadline {
                    return;
                }
                if native_effects {
                    upcoming_programs(&self.effects, &mut saved, layer, seconds, &mut programs);
                }
                if kind == "shader" {
                    shaders.push(layer);
                }
                if !matches!(kind, "image" | "video") {
                    return;
                }
                let data = member(Some(layer), "d");
                let Some(asset) = string(member(data, "asset")).and_then(|s| s.as_str()) else {
                    return;
                };
                let Some(record) = member(member(Some(comp.root()), "assets"), asset) else {
                    return;
                };
                if kind == "image" && is_str(member(Some(record), "format"), "svg") {
                    return;
                }
                let Some(path) = assets.path(asset, Some(record)) else {
                    return;
                };
                let interpret = crate::footage::interpretation(record);
                if kind == "image" {
                    let edge = upcoming_edge(layer, record, seconds, scale);
                    if !request.export && stills.insert((asset, edge)) {
                        self.media.prepare_image(&path, edge, interpret.as_ref());
                    }
                } else if let Some(at) = upcoming_source_time(layer, seconds) {
                    let id =
                        pm_eval::node::to_js_string(member(Some(layer), "id")).to_string_lossy();
                    let instance = crate::exec::instance_of(&id);
                    let rate = comp.rate().map_or(pm_time::Rate::FPS_30, |r| r.rate);
                    result.videos.push((id, instance));
                    if self
                        .media
                        .prepare_video(
                            &path,
                            instance,
                            at.max(0.),
                            request.export,
                            request.read_ahead,
                            interpret.as_ref(),
                            rate,
                        )
                        .unwrap_or(false)
                    {
                        result.queued_videos += 1;
                    }
                }
            },
        );
        if !programs.is_empty() || !shaders.is_empty() {
            let precision = project_settings(comp.root())?.precision;
            let format = pm_gpu::target::working_format(precision);
            self.shaders.prewarm(
                &self.device,
                format,
                shaders
                    .into_iter()
                    .rev()
                    .filter_map(crate::shader_layers::exec::prewarm_source),
                true,
            );
            self.effects
                .prewarm_selected(&self.device, format, &programs);
        }
        Ok(result)
    }

    /// Wait for queued preparation in headless tests and performance runs.
    /// The interactive presenter must use `prepare_media` alone.
    pub fn finish_media_preparation(&mut self) -> Vec<String> {
        self.media.wait_for_stills();
        self.media
            .wait_for_preparation()
            .into_iter()
            .map(|error| format!("{error:?}"))
            .collect()
    }
}

#[cfg(test)]
mod upcoming_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shader_lookahead_follows_the_active_precomps_source_time() {
        let doc = Document::new(Some(pm_model::json::parse(&json!({
            "id":"root","layers":[
                {"id":"late","type":"shader","on":true,"from":5,"dur":2,"d":{"code":"// late"}},
                {"id":"nested","type":"precomp","on":true,"from":1,"dur":3,"d":{"comp":"child","speed":2,"trim":4}}
            ],
            "comps":{"child":{"layers":[
                {"id":"early","type":"shader","on":true,"from":6,"dur":2,"d":{"code":"// early"}}
            ]}}
        }).to_string()).unwrap()), 1);
        let selected = program_layers(&doc, "root")
            .into_iter()
            .filter_map(crate::shader_layers::exec::prewarm_source)
            .map(|(_, source, _)| source)
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 2);
        assert!(selected[0].contains("// early"));
        let mut upcoming = Vec::new();
        let comp = Comp::resolve(&doc, "root").unwrap();
        visit_upcoming(
            &comp,
            2.,
            (1., 1.),
            0,
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            &mut |_, layer, _, _, _| {
                upcoming.extend(crate::shader_layers::exec::prewarm_source(layer))
            },
        );
        assert_eq!(upcoming.len(), 1);
        assert!(upcoming[0].1.contains("// early"));
    }

    #[test]
    fn lookahead_selects_programs_for_enabled_layers_at_the_cut() {
        let doc = Document::new(Some(pm_model::json::parse(&json!({
            "id":"root","layers":[
                {"type":"text","on":true,"from":2,"dur":1,"fx":[
                    {"type":"blur","on":true},{"type":"blur","on":true},{"type":"glow","on":false}
                ]},
                {"type":"solid","on":true,"from":0,"dur":1,"fx":[{"type":"fill","on":true}]}
            ]
        }).to_string()).unwrap()), 1);
        let comp = Comp::resolve(&doc, "root").unwrap();
        let mut programs = Vec::new();
        let effects = crate::effects::Effects::new("lookahead-test");
        let mut saved = crate::effects::prewarm::SavedEffects::default();
        visit_upcoming(
            &comp,
            2.,
            (1., 1.),
            0,
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            &mut |_, layer, _, time, _| {
                upcoming_programs(&effects, &mut saved, layer, time, &mut programs)
            },
        );
        assert_eq!(programs.len(), 1);
        assert_eq!(programs[0].1.as_str(), "blur");
    }

    #[test]
    fn large_scene_finds_active_stills_and_retimed_nested_video() {
        let mut layers = vec![json!({"type":"shape","on":true,"from":0,"dur":4}); 600];
        layers.extend([
            json!({"type":"image","on":false,"from":0,"dur":4,"id":"off"}),
            json!({"type":"image","on":true,"from":0,"dur":1,"id":"ended"}),
            json!({"type":"image","on":true,"from":1,"dur":1,"id":"still"}),
            json!({"type":"precomp","on":true,"from":1,"dur":2,"d":{
                "comp":"child","timeRemap":true,"sourceTime":{"v":4,"kf":[]}
            }}),
        ]);
        let project = json!({
            "id":"root","layers":layers,
            "comps":{"child":{"layers":[
                {"type":"video","on":true,"from":3,"dur":2,"id":"nested","d":{"speed":2,"trim":0.5}},
                {"type":"image","on":true,"from":0,"dur":2,"id":"outside"}
            ]}}
        });
        let doc = Document::new(
            Some(pm_model::json::parse(&project.to_string()).unwrap()),
            1,
        );
        let comp = Comp::resolve(&doc, "root").unwrap();
        let mut found = Vec::new();
        let mut scan = |_: &Comp<'_>, layer: &pm_model::Node, kind: &str, time, _| {
            if kind == "precomp" {
                return;
            }
            let id = pm_eval::node::to_js_string(layer.get("id")).to_string_lossy();
            found.push((id, kind.to_owned(), upcoming_source_time(layer, time)));
        };
        let now = std::time::Instant::now();
        visit_upcoming(&comp, 1., (1., 1.), 0, now, &mut scan);
        visit_upcoming(
            &comp,
            1.,
            (1., 1.),
            0,
            now + std::time::Duration::from_secs(1),
            &mut scan,
        );
        assert_eq!(
            found,
            [
                ("still".into(), "image".into(), Some(0.)),
                ("nested".into(), "video".into(), Some(2.5))
            ]
        );
    }
}
