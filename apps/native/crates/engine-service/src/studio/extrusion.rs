//! Text geometry uses the native font broker and the renderer's glyph outlines.
use super::*;
use pm_eval::{Comp, Evaluator, JsValue, Scope};
use pm_scene3d::extrusion::{self, Bevel, Options};
fn json(value: JsValue) -> Result<Value, String> {
    match value {
        JsValue::Number(n) => Ok(serde_json::json!(n)),
        JsValue::Bool(b) => Ok(serde_json::json!(b)),
        JsValue::String(s) => Ok(serde_json::json!(s.to_string_lossy())),
        _ => Err("Invalid evaluated extrusion value".into()),
    }
}
impl DocumentSource {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_extrusions(
        &self,
        request: &SceneRequest,
        scene: &mut Scene,
        instances: &mut Vec<MeshInstance>,
        materials: &mut Vec<GpuMaterial>,
        maps: &mut Vec<Vec<u8>>,
        indices: &mut HashMap<(String, usize), usize>,
        failures: &mut Vec<pm_render::LayerFailure>,
    ) -> Result<(), String> {
        let comp = Comp::resolve(&self.snapshot.document, &request.composition)
            .map_err(|e| e.to_string())?;
        let mut ev = Evaluator::new().with_expressions(Arc::new(pm_render::expr::PmExprEngine));
        let mut scope = Scope::open(&mut ev, &comp);
        let time = request.ticks as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
        let width = pm_eval::node::number(comp.member("w")).unwrap_or(1.);
        let height = pm_eval::node::number(comp.member("h"))
            .unwrap_or(1.)
            .max(1.);
        let unit = 6. / height;
        let bridge = pm_scene3d::render::gltf::math::compose(
            [-width * unit / 2., 3., 0.],
            [0., 0., 0., 1.],
            [unit, -unit, -unit],
        );
        let flip =
            pm_scene3d::render::gltf::math::compose([0.; 3], [0., 0., 0., 1.], [1., -1., -1.]);
        let layers: Vec<_> = scope.layers().iter().collect();
        for layer in layers {
            let stored = pm_scene3d::from_model(&layer.to_value()).map_err(|e| e.to_string())?;
            if !extrusion::enabled(&stored) {
                continue;
            }
            let id = stored["id"].as_str().ok_or("Extruded layer has no ID")?;
            let checkpoint = (
                instances.len(),
                materials.len(),
                scene.materials.len(),
                maps.len(),
            );
            let result = (|| -> Result<(), String> {
                if !scope.active(&layer, time).map_err(|e| e.message)? {
                    return Ok(());
                }
                let mut data =
                    extrusion::parse(&stored["d"]["extrusion"]).map_err(|e| e.to_string())?;
                let channels: Vec<_> = extrusion::channels(
                    &serde_json::json!({"type":stored["type"],"d":{"extrusion":data}}),
                )
                .into_iter()
                .map(|(k, v)| (k, v.clone()))
                .collect();
                for (path, channel) in channels {
                    let prop = pm_eval::address::find_prop(
                        &layer,
                        &pm_model::JsString::from(path.as_str()),
                    )
                    .map_err(|e| e.message)?;
                    let value = if let Some(prop) = prop.node() {
                        json(
                            scope
                                .evaluated(&layer, Some(prop), time, &path)
                                .map_err(|e| e.message)?,
                        )?
                    } else {
                        channel["v"].clone()
                    };
                    let parts: Vec<_> = path.split('.').collect();
                    if parts[0] == "geo" {
                        pm_scene3d::schema::validate_channel("extrusion", parts[1], value.clone())
                            .map_err(|e| e.to_string())?;
                        data["p"][parts[1]] = value;
                    } else {
                        pm_scene3d::schema::validate_channel("surface", parts[2], value.clone())
                            .map_err(|e| e.to_string())?;
                        data["surfaces"][parts[1]]["properties"][parts[2]] = value;
                    }
                }
                let p = &data["p"];
                let bevel = match val(&p["bevelStyle"]).as_str().unwrap_or("none") {
                    "angular" => Bevel::Angular,
                    "concave" => Bevel::Concave,
                    "convex" => Bevel::Convex,
                    _ => Bevel::None,
                };
                let (steps, tolerance) = match val(&p["quality"]).as_str().unwrap_or("standard") {
                    "draft" => (3, 0.4),
                    "high" => (12, 0.05),
                    _ => (6, 0.15),
                };
                let outlines = if stored["type"] == "text" {
                    self.text
                        .lock()
                        .map_err(|_| "Text extrusion cache poisoned")?
                        .extrusion_outlines(&mut scope, &layer, time, tolerance)?
                } else {
                    pm_render::vector::extrusion_outlines(&mut scope, &layer, time, tolerance)?
                };
                if outlines.is_empty() {
                    return Ok(());
                }
                let mut digest = blake3::Hasher::new();
                digest.update(p.to_string().as_bytes());
                for outline in &outlines {
                    for c in outline.color {
                        digest.update(&c.to_le_bytes());
                    }
                    for ring in std::iter::once(&outline.outer).chain(outline.holes.iter()) {
                        digest.update(&(ring.len() as u64).to_le_bytes());
                        for point in ring {
                            for x in point {
                                digest.update(&x.to_le_bytes());
                            }
                        }
                    }
                }
                let key = format!("extrusion:{}", digest.finalize().to_hex());
                let built = {
                    let mut cache = self
                        .extruded
                        .lock()
                        .map_err(|_| "Extrusion cache poisoned")?;
                    let built = if let Some(index) = cache.iter().position(|(k, _)| k == &key) {
                        cache.remove(index).1
                    } else {
                        Arc::new(extrusion::build(
                            &outlines,
                            Options {
                                depth: n(p, "depth", 0.) as f64,
                                bevel,
                                bevel_depth: n(p, "bevelDepth", 2.) as f64,
                                hole_bevel_depth: n(p, "holeBevelDepth", 100.) as f64,
                                steps,
                            },
                        )?)
                    };
                    cache.push((key.clone(), built.clone()));
                    while cache.len() > 1
                        && (cache.len() > 8
                            || cache.iter().map(|(_, g)| g.colors.len()).sum::<usize>() > 1_000_000)
                    {
                        cache.remove(0);
                    }
                    built
                };
                let mesh = self.cached_mesh(key, || {
                    let mut mesh = convert_mesh(&built.geometry);
                    for group in &built.geometry.groups {
                        mesh.triangle_materials.extend(std::iter::repeat_n(
                            group.material_index as usize,
                            group.count as usize / 3,
                        ));
                    }
                    Ok(mesh)
                })?;
                let mut colors = built.colors.clone();
                for color in &mut colors {
                    for c in &mut color[..3] {
                        *c = if *c <= 0.04045 {
                            *c / 12.92
                        } else {
                            ((*c + 0.055) / 1.055).powf(2.4)
                        };
                    }
                }
                let mut palette = vec![];
                for (index, surface) in extrusion::SURFACES.iter().enumerate() {
                    let material = &data["surfaces"][surface];
                    let properties = &material["properties"];
                    let mut m = Material::dielectric(
                        color(properties, "color", "#FFFFFF"),
                        n(properties, "roughness", 0.4),
                    );
                    m.metallic = n(properties, "metallic", 0.);
                    m.emission = color(properties, "emissionColor", "#000000")
                        * n(properties, "emissionIntensity", 0.);
                    let mut gpu = m.gpu();
                    gpu.spec[0] = m.roughness.max(0.002);
                    gpu.extra[2] = n(properties, "opacity", 100.) / 100.;
                    gpu.policy[0] = n(properties, "ambientResponse", 100.) / 100.;
                    self.bind_material_maps(material, &mut gpu, maps, indices)?;
                    gpu.validate()?;
                    palette.push(materials.len());
                    materials.push(gpu);
                    scene.materials.push(m);
                    if properties.get("color").is_some() || material["maps"].get("color").is_some()
                    {
                        let group = built.geometry.groups[index];
                        for color in
                            &mut colors[group.start as usize..(group.start + group.count) as usize]
                        {
                            color[..3].fill(1.);
                        }
                    }
                }
                let attributes = pm_studio3d_engine::extract::VertexAttributes {
                    colors: Arc::new(colors),
                    ..Default::default()
                }
                .shared()?;
                let world = scope.world3d(&layer, time, false).map_err(|e| e.message)?;
                let world = pm_scene3d::render::gltf::math::multiply(
                    &bridge,
                    &pm_scene3d::render::gltf::math::multiply(&world, &flip),
                );
                let world = Transform(world.map(|v| v as f32));
                world.validate()?;
                instances.push(MeshInstance {
                    layer: id.into(),
                    mesh,
                    material: palette[0],
                    material_palette: Some(Arc::new(palette)),
                    attributes: Some(attributes),
                    primary_visible: true,
                    world,
                });
                Ok(())
            })();
            if let Err(message) = result {
                instances.truncate(checkpoint.0);
                materials.truncate(checkpoint.1);
                scene.materials.truncate(checkpoint.2);
                maps.truncate(checkpoint.3);
                indices.retain(|_, i| *i < checkpoint.3);
                failures.push(pm_render::LayerFailure {
                    composition: request.composition.clone(),
                    layer: id.into(),
                    message,
                });
            }
        }
        Ok(())
    }
}
