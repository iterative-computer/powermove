//! Native viewport chrome inputs, evaluated in the same document/time/view
//! as the color frame. No renderer-side scene reconstruction is involved.
use super::*;
use pm_eval::{JsValue, Scope, node, parenting::Role3d};
use pm_protocol::frame::{ChromeGlyph, ChromeSelection, ViewOverride};

impl Renderer {
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn render_viewport_chrome(
        &mut self,
        document: &Document,
        composition: &str,
        time: Ticks,
        assets: &dyn AssetResolver,
        view: Option<&ViewOverride>,
        selection: Option<&ChromeSelection>,
        size: (u32, u32),
        rect: [f64; 4],
        depth: &wgpu::TextureView,
        coverage: &wgpu::TextureView,
    ) -> Result<(Vec<String>, Vec<ChromeGlyph>, ChromeSelection), RenderError> {
        let comp = Comp::resolve(document, composition)
            .map_err(|_| RenderError::Composition(composition.into()))?;
        let seconds = comp.seconds_at(time.get());
        let number = |key| node::to_number(comp.member(key)).max(1.0).round() as u32;
        let (width, height) = (number("w"), number("h"));
        let mut scope = Scope::new(&mut self.evaluator, &comp, &comp);
        scope.memoize_transforms();
        let view3d = view.map(crate::scene3d::View3d::from_override);
        let mut scene = crate::scene3d::layers::composition_scene(
            &mut scope,
            seconds,
            width,
            height,
            assets,
            view3d.as_ref(),
            true,
        )
        .map_err(|e| RenderError::Request(format!("viewport geometry: {e:?}")))?
        .map_err(RenderError::Request)?;
        let local = view3d.as_ref().and_then(|v| v.local.as_ref());
        if let Some(local) = local {
            for object in &mut scene.objects {
                object.visible &= local.contains(&object.id);
            }
        }
        let mut chosen = selection.cloned().unwrap_or_default();
        let layers: Vec<_> = scope.layers().iter().collect();
        for object in &scene.objects {
            let Some(layer) = layers
                .iter()
                .copied()
                .find(|l| crate::js::layer_id(l) == object.id)
            else {
                continue;
            };
            let ancestors = scope
                .group_ancestors(layer)
                .map_err(|e| RenderError::Request(format!("viewport groups: {e:?}")))?;
            for group in ancestors {
                if !scope
                    .is_model_group(group)
                    .map_err(|e| RenderError::Request(format!("viewport group: {e:?}")))?
                {
                    continue;
                }
                let id = crate::js::layer_id(group);
                if chosen.selected.contains(&id) && !chosen.selected.contains(&object.id) {
                    chosen.selected.push(object.id.clone());
                }
                if chosen.active.contains(&id) && !chosen.active.contains(&object.id) {
                    chosen.active.push(object.id.clone());
                }
            }
        }
        let mut glyphs = Vec::new();
        for layer in layers {
            let role = Role3d::of(layer);
            if !matches!(role, Some(Role3d::Camera | Role3d::Light)) {
                continue;
            }
            let id = crate::js::layer_id(layer);
            if local.is_some_and(|ids| !ids.contains(&id))
                || !scope
                    .active(layer, seconds)
                    .map_err(|e| RenderError::Request(format!("viewport glyph: {e:?}")))?
            {
                continue;
            }
            let name = if role == Some(Role3d::Camera) {
                "camera"
            } else {
                "light"
            };
            let data = node::member(node::member(node::member(Some(layer), "d"), "data"), name);
            let props = node::member(data, "p");
            let world = scope
                .world3d(layer, seconds, false)
                .map_err(|e| RenderError::Request(format!("viewport glyph world: {e:?}")))?;
            let target = scope
                .world_aim3d(layer, seconds)
                .map_err(|e| RenderError::Request(format!("viewport glyph aim: {e:?}")))?;
            let mut value = |key: &str, fallback: f64| -> Result<f64, RenderError> {
                let v = scope
                    .evaluated(
                        layer,
                        node::member(props, key),
                        seconds,
                        &format!("{name}.{key}"),
                    )
                    .map_err(|e| RenderError::Request(format!("viewport glyph channel: {e:?}")))?;
                Ok(match v {
                    JsValue::Number(n) if n.is_finite() => n,
                    _ => fallback,
                })
            };
            let kind = if name == "camera" {
                "camera".into()
            } else {
                node::string(node::member(data, "type"))
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_else(|| "point".into())
            };
            glyphs.push(ChromeGlyph {
                id,
                kind,
                position: [world[12], world[13], world[14]],
                target,
                up: crate::three::math::normalize([world[4], world[5], world[6]]),
                fov: value("fov", 50.0)?,
                aspect: f64::from(width) / f64::from(height),
                angle: value("angle", 30.0)?,
                size: [value("width", 1.0)?, value("height", 1.0)?],
            });
        }
        let legend = scene.objects.iter().map(|o| o.id.clone()).collect();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("native viewport chrome"),
            });
        self.scene3d
            .render_chrome(
                &self.device,
                &self.queue,
                &mut encoder,
                &scene,
                size,
                rect,
                &chosen.selected,
                &chosen.active,
                depth,
                coverage,
            )
            .map_err(|e| RenderError::Request(format!("viewport chrome: {e}")))?;
        self.queue.submit([encoder.finish()]);
        Ok((legend, glyphs, chosen))
    }
}
