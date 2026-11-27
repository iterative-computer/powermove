//! Three r180 `GLTFParser.loadNode` attachment order for shared nodes.
//!
//! `Object3D.add` moves a shared child from its old parent. The winner is
//! decided by the order of Promise reactions, not by tree depth or JSON order.
//! This small scheduler mirrors the dependency/cache and Promise chains used
//! by the loader's node, mesh, geometry, accessor, skin, and scene methods.

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

fn id(v: &Value) -> Option<usize> {
    v.as_u64().map(|n| n as usize)
}
fn members(v: &Value) -> impl Iterator<Item = usize> + '_ {
    v.as_array().into_iter().flatten().filter_map(id)
}

fn js_key(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
    }
}

fn attributes_key(v: &Value) -> String {
    let mut out = String::new();
    if let Some(attrs) = v.as_object() {
        let mut keys: Vec<_> = attrs.keys().collect();
        keys.sort();
        for key in keys {
            out.push_str(key);
            out.push(':');
            out.push_str(&js_key(attrs.get(key)));
            out.push(';');
        }
    }
    out
}

/// r180 `createPrimitiveKey`, including its JavaScript `undefined` strings.
fn primitive_key(def: &Value) -> String {
    let mut key = format!(
        "{}:{}:{}",
        js_key(def.get("indices")),
        attributes_key(&def["attributes"]),
        js_key(def.get("mode"))
    );
    if let Some(targets) = def["targets"].as_array() {
        for target in targets {
            key.push(':');
            key.push_str(&attributes_key(target));
        }
    }
    key
}

type P = usize;

#[derive(Clone)]
enum Job {
    Resolve(P),
    AllMember(P),
    StartImage(P),
    Attach {
        node: usize,
        done: P,
    },
    AttachScene {
        scene: usize,
        roots: Vec<usize>,
        done: P,
    },
    Morph {
        primitive: Value,
        done: P,
    },
}

struct Promise {
    settled: bool,
    remaining: usize,
    reactions: Vec<Job>,
}

struct Schedule<'a> {
    json: &'a Value,
    promises: Vec<Promise>,
    microtasks: VecDeque<Job>,
    // FileLoader/data URI and decoded-image completions are task boundaries:
    // every queued Promise reaction drains before the next completion.
    external: Vec<P>,
    cache: HashMap<(&'static str, usize), P>,
    shallow: HashMap<usize, P>,
    geometry: HashMap<String, P>,
    images: HashMap<usize, P>,
    texture_images: HashMap<String, P>,
    parents: Vec<Option<usize>>,
    root_owner: Vec<Option<usize>>,
    bad: bool,
}

impl<'a> Schedule<'a> {
    fn new(json: &'a Value, nodes: usize) -> Self {
        Self {
            json,
            promises: Vec::new(),
            microtasks: VecDeque::new(),
            external: Vec::new(),
            cache: HashMap::new(),
            shallow: HashMap::new(),
            geometry: HashMap::new(),
            images: HashMap::new(),
            texture_images: HashMap::new(),
            parents: vec![None; nodes],
            root_owner: vec![None; nodes],
            bad: false,
        }
    }

    fn pending(&mut self) -> P {
        let i = self.promises.len();
        self.promises.push(Promise {
            settled: false,
            remaining: 0,
            reactions: Vec::new(),
        });
        i
    }

    fn ready(&mut self) -> P {
        let p = self.pending();
        self.resolve(p);
        p
    }

    fn resolve(&mut self, p: P) {
        if self.promises[p].settled {
            return;
        }
        self.promises[p].settled = true;
        self.microtasks
            .extend(std::mem::take(&mut self.promises[p].reactions));
    }

    fn on(&mut self, src: P, job: Job) {
        if self.promises[src].settled {
            self.microtasks.push_back(job);
        } else {
            self.promises[src].reactions.push(job);
        }
    }

    fn then(&mut self, src: P) -> P {
        let done = self.pending();
        self.on(src, Job::Resolve(done));
        done
    }

    fn all(&mut self, sources: &[P]) -> P {
        let done = self.pending();
        if sources.is_empty() {
            self.resolve(done);
        } else {
            self.promises[done].remaining = sources.len();
            for &src in sources {
                self.on(src, Job::AllMember(done));
            }
        }
        done
    }

    fn run(&mut self) {
        while !self.microtasks.is_empty() || !self.external.is_empty() {
            while let Some(job) = self.microtasks.pop_front() {
                match job {
                    Job::Resolve(p) => self.resolve(p),
                    Job::AllMember(p) => {
                        self.promises[p].remaining -= 1;
                        if self.promises[p].remaining == 0 {
                            self.resolve(p);
                        }
                    }
                    Job::StartImage(p) => self.external.push(p),
                    Job::Attach { node, done } => {
                        for child in members(&self.json["nodes"][node]["children"]) {
                            if child >= self.parents.len() {
                                self.bad = true;
                                continue;
                            }
                            self.parents[child] = Some(node);
                            self.root_owner[child] = None;
                        }
                        self.resolve(done);
                    }
                    Job::AttachScene { scene, roots, done } => {
                        for root in roots {
                            if root >= self.parents.len() {
                                self.bad = true;
                                continue;
                            }
                            self.parents[root] = None;
                            self.root_owner[root] = Some(scene);
                        }
                        self.resolve(done);
                    }
                    Job::Morph { primitive, done } => {
                        let p = self.morph(&primitive);
                        self.on(p, Job::Resolve(done));
                    }
                }
            }
            if !self.external.is_empty() {
                let p = self.external.remove(0);
                self.resolve(p);
            }
        }
    }

    fn node(&mut self, i: usize) -> P {
        if let Some(&p) = self.cache.get(&("node", i)) {
            return p;
        }
        let done = self.pending();
        self.cache.insert(("node", i), done);
        let def = self.json["nodes"][i].clone();
        let shallow = self.shallow(i);
        let children: Vec<P> = members(&def["children"]).map(|j| self.node(j)).collect();
        let children = self.all(&children);
        let skin = match id(&def["skin"]) {
            Some(j) => self.skin(j),
            None => self.ready(),
        };
        let all = self.all(&[shallow, children, skin]);
        self.on(all, Job::Attach { node: i, done });
        done
    }

    fn shallow(&mut self, i: usize) -> P {
        if let Some(&p) = self.shallow.get(&i) {
            return p;
        }
        let def = self.json["nodes"][i].clone();
        let mut pending = Vec::new();
        if let Some(j) = id(&def["mesh"]) {
            let instancing = def["extensions"]["EXT_mesh_gpu_instancing"]["attributes"].as_object();
            let supported = self.json["meshes"][j]["primitives"]
                .as_array()
                .is_some_and(|parts| {
                    parts
                        .iter()
                        .all(|p| p["mode"].as_u64().is_none_or(|mode| matches!(mode, 4..=6)))
                });
            if let Some(attrs) = instancing.filter(|attrs| !attrs.is_empty() && supported) {
                let mut instance_deps: Vec<P> = attrs
                    .values()
                    .filter_map(id)
                    .map(|index| {
                        let accessor = self.accessor(index);
                        self.then(accessor)
                    })
                    .collect();
                let mesh = self.mesh(j);
                let source = self.then(mesh); // parser.createNodeMesh
                instance_deps.push(source);
                let all = self.all(&instance_deps);
                pending.push(self.then(all)); // EXT_mesh_gpu_instancing.createNodeMesh
            } else {
                let mesh = self.mesh(j);
                pending.push(self.then(mesh));
            }
        }
        if let Some(j) = id(&def["camera"]) {
            let camera = self.camera(j);
            pending.push(self.then(camera));
        }
        // KHR_lights_punctual.createNodeAttachment returns loadLight().then.
        if let Some(j) = id(&def["extensions"]["KHR_lights_punctual"]["light"]) {
            let light = self.camera(j);
            pending.push(self.then(light));
        }
        let all = self.all(&pending);
        let done = self.then(all);
        self.shallow.insert(i, done);
        done
    }

    fn camera(&mut self, i: usize) -> P {
        if let Some(&p) = self.cache.get(&("camera", i)) {
            return p;
        }
        let p = self.ready();
        self.cache.insert(("camera", i), p);
        p
    }

    fn skin(&mut self, i: usize) -> P {
        if let Some(&p) = self.cache.get(&("skin", i)) {
            return p;
        }
        let def = self.json["skins"][i].clone();
        let mut pending: Vec<P> = members(&def["joints"]).map(|j| self.shallow(j)).collect();
        pending.push(match id(&def["inverseBindMatrices"]) {
            Some(j) => self.accessor(j),
            None => self.ready(),
        });
        let all = self.all(&pending);
        let done = self.then(all);
        self.cache.insert(("skin", i), done);
        done
    }

    fn mesh(&mut self, i: usize) -> P {
        if let Some(&p) = self.cache.get(&("mesh", i)) {
            return p;
        }
        let primitives = self.json["meshes"][i]["primitives"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut pending: Vec<P> = primitives
            .iter()
            .map(|p| match id(&p["material"]) {
                Some(j) => self.material(j),
                None => self.ready(),
            })
            .collect();
        let geoms: Vec<P> = primitives.iter().map(|p| self.primitive(p)).collect();
        pending.push(self.all(&geoms));
        let all = self.all(&pending);
        let done = self.then(all);
        self.cache.insert(("mesh", i), done);
        done
    }

    fn primitive(&mut self, def: &Value) -> P {
        let key = primitive_key(def);
        if let Some(&p) = self.geometry.get(&key) {
            return p;
        }
        let mut pending = Vec::new();
        if let Some(attrs) = def["attributes"].as_object() {
            for j in attrs.values().filter_map(id) {
                let a = self.accessor(j);
                pending.push(self.then(a));
            }
        }
        if let Some(j) = id(&def["indices"]) {
            let a = self.accessor(j);
            pending.push(self.then(a));
        }
        let all = self.all(&pending);
        let done = self.pending();
        if def.get("targets").is_some() {
            self.on(
                all,
                Job::Morph {
                    primitive: def.clone(),
                    done,
                },
            );
        } else {
            self.on(all, Job::Resolve(done));
        }
        self.geometry.insert(key, done);
        done
    }

    fn morph(&mut self, def: &Value) -> P {
        let targets = def["targets"].as_array().cloned().unwrap_or_default();
        let mut sets = Vec::new();
        for name in ["POSITION", "NORMAL", "COLOR_0"] {
            if targets.iter().any(|t| t.get(name).is_some()) {
                let deps: Vec<P> = targets
                    .iter()
                    .map(|t| match id(&t[name]) {
                        Some(j) => self.accessor(j),
                        None => self.ready(),
                    })
                    .collect();
                sets.push(self.all(&deps));
            } else {
                sets.push(self.all(&[]));
            }
        }
        let all = self.all(&sets);
        self.then(all)
    }

    fn accessor(&mut self, i: usize) -> P {
        if let Some(&p) = self.cache.get(&("accessor", i)) {
            return p;
        }
        let def = self.json["accessors"][i].clone();
        let mut pending = Vec::new();
        if let Some(j) = id(&def["bufferView"]) {
            pending.push(self.view(j));
        } else if def.get("sparse").is_some() {
            pending.push(self.ready());
        }
        if let Some(j) = id(&def["sparse"]["indices"]["bufferView"]) {
            pending.push(self.view(j));
        }
        if let Some(j) = id(&def["sparse"]["values"]["bufferView"]) {
            pending.push(self.view(j));
        }
        let done = if pending.is_empty() {
            self.ready()
        } else {
            let all = self.all(&pending);
            self.then(all)
        };
        self.cache.insert(("accessor", i), done);
        done
    }

    fn view(&mut self, i: usize) -> P {
        if let Some(&p) = self.cache.get(&("bufferView", i)) {
            return p;
        }
        let b = id(&self.json["bufferViews"][i]["buffer"]).unwrap_or(0);
        let source = self.buffer(b);
        let done = self.then(source);
        self.cache.insert(("bufferView", i), done);
        done
    }

    fn buffer(&mut self, i: usize) -> P {
        if let Some(&p) = self.cache.get(&("buffer", i)) {
            return p;
        }
        let p = if self.json["buffers"][i].get("uri").is_none() && i == 0 {
            self.ready()
        } else {
            let p = self.pending();
            self.external.push(p);
            p
        };
        self.cache.insert(("buffer", i), p);
        p
    }

    fn material(&mut self, i: usize) -> P {
        if let Some(&p) = self.cache.get(&("material", i)) {
            return p;
        }
        let def = self.json["materials"][i].clone();
        let unlit = def["extensions"].get("KHR_materials_unlit").is_some()
            && self.json["extensionsUsed"]
                .as_array()
                .is_some_and(|extensions| {
                    extensions
                        .iter()
                        .any(|v| v.as_str() == Some("KHR_materials_unlit"))
                });
        let pbr = &def["pbrMetallicRoughness"];
        let mut pending = Vec::new();
        if let Some(p) = self.assign_texture(&pbr["baseColorTexture"]) {
            pending.push(p);
        }
        if unlit {
            // GLTFMaterialsUnlitExtension.extendParams returns Promise.all,
            // including for a texture-free material.
            let unlit_pending = std::mem::take(&mut pending);
            pending.push(self.all(&unlit_pending));
        } else {
            if let Some(p) = self.assign_texture(&pbr["metallicRoughnessTexture"]) {
                pending.push(p);
            }
            if let Some(p) = self.assign_texture(&pbr["metallicRoughnessTexture"]) {
                pending.push(p);
            }
            // All eleven built-in material plugins are registered even when
            // their extensions are absent. Each extendMaterialParams returns
            // a resolved promise; _invokeAll's Promise.all of those promises
            // adds a real reaction checkpoint before the outer material all.
            const PLUGINS: [(&str, &[&str]); 11] = [
                (
                    "KHR_materials_clearcoat",
                    &[
                        "clearcoatTexture",
                        "clearcoatRoughnessTexture",
                        "clearcoatNormalTexture",
                    ],
                ),
                ("KHR_materials_dispersion", &[]),
                (
                    "KHR_materials_sheen",
                    &["sheenColorTexture", "sheenRoughnessTexture"],
                ),
                ("KHR_materials_transmission", &["transmissionTexture"]),
                ("KHR_materials_volume", &["thicknessTexture"]),
                ("KHR_materials_ior", &[]),
                ("KHR_materials_emissive_strength", &[]),
                (
                    "KHR_materials_specular",
                    &["specularTexture", "specularColorTexture"],
                ),
                (
                    "KHR_materials_iridescence",
                    &["iridescenceTexture", "iridescenceThicknessTexture"],
                ),
                ("KHR_materials_anisotropy", &["anisotropyTexture"]),
                ("EXT_materials_bump", &["bumpTexture"]),
            ];
            let plugins: Vec<P> = PLUGINS
                .iter()
                .map(|(name, slots)| {
                    let ext = &def["extensions"][*name];
                    if ext.is_null() {
                        return self.ready();
                    }
                    let deps: Vec<P> = slots
                        .iter()
                        .filter_map(|slot| self.assign_texture(&ext[*slot]))
                        .collect();
                    if slots.is_empty() {
                        self.ready()
                    } else {
                        self.all(&deps)
                    }
                })
                .collect();
            pending.push(self.all(&plugins));
            for slot in ["normalTexture", "occlusionTexture", "emissiveTexture"] {
                if let Some(p) = self.assign_texture(&def[slot]) {
                    pending.push(p);
                }
            }
        }
        let all = self.all(&pending);
        let done = self.then(all);
        self.cache.insert(("material", i), done);
        done
    }

    fn assign_texture(&mut self, slot: &Value) -> Option<P> {
        let index = id(&slot["index"])?;
        let texture = self.texture(index);
        Some(self.then(texture))
    }

    fn texture(&mut self, i: usize) -> P {
        if let Some(&p) = self.cache.get(&("texture", i)) {
            return p;
        }
        let def = self.json["textures"][i].clone();
        let source = id(&def["source"])
            .or_else(|| id(&def["extensions"]["EXT_texture_webp"]["source"]))
            .or_else(|| id(&def["extensions"]["EXT_texture_avif"]["source"]));
        let Some(j) = source else {
            let p = self.ready();
            self.cache.insert(("texture", i), p);
            return p;
        };
        let image = &self.json["images"][j];
        // GLTFParser.loadTextureImage caches by `(uri || bufferView) + ':' +
        // sampler`, independently of the getDependency(texture) index cache.
        let key = format!(
            "{}:{}",
            image["uri"]
                .as_str()
                .filter(|uri| !uri.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| js_key(image.get("bufferView"))),
            js_key(def.get("sampler"))
        );
        let p = if let Some(&cached) = self.texture_images.get(&key) {
            cached
        } else {
            let source = self.image(j);
            let configured = self.then(source); // flipY, name, sampler, associations
            let done = self.then(configured); // catch(null) on the fulfilled path
            self.texture_images.insert(key, done);
            done
        };
        let done = p;
        self.cache.insert(("texture", i), done);
        done
    }

    fn image(&mut self, i: usize) -> P {
        if let Some(&p) = self.images.get(&i) {
            return self.then(p); // sourceCache clone for another sampler
        }
        let def = self.json["images"][i].clone();
        let uri = match id(&def["bufferView"]) {
            // Creating the Blob URL is the bufferView.then callback. The
            // following Promise.resolve(sourceURI).then starts the loader.
            Some(j) => {
                let view = self.view(j);
                self.then(view)
            }
            None => self.ready(),
        };
        let decoded = self.pending();
        // The decoded-image policy resolves ImageBitmapLoader's onLoad in a
        // task, in request order. Even a GLB-embedded image needs that task:
        // loadImageSource creates a fresh Blob URL and calls loader.load only
        // after the bufferView/URI Promise chain. Live decoder completion can
        // race and is deliberately outside this deterministic policy.
        self.on(uri, Job::StartImage(decoded));
        let adopted = self.then(decoded); // outer Promise adopts the loader Promise
        let configured = self.then(adopted); // source extras and mimeType
        let done = self.then(configured); // catch on the fulfilled path
        self.images.insert(i, done);
        done
    }
}

/// Result of all `loadScene` attachment callbacks. An Object3D can move
/// between scenes just as it can move between parent nodes.
pub(crate) struct Selection {
    pub parents: Vec<Option<usize>>,
    pub default_roots: Vec<usize>,
}

pub(crate) fn selected_parents(json: &Value) -> Option<Selection> {
    let nodes = json["nodes"].as_array()?;
    let scene = id(&json["scene"]).unwrap_or(0);
    let scenes = json["scenes"].as_array()?;
    let default = scenes.get(scene)?;
    let default_roots: Vec<usize> = members(&default["nodes"]).collect();
    if default_roots.iter().any(|&i| i >= nodes.len()) {
        return None;
    }
    let mut schedule = Schedule::new(json, nodes.len());
    let mut scene_promises = Vec::new();
    for (index, def) in scenes.iter().enumerate() {
        let roots: Vec<usize> = members(&def["nodes"]).collect();
        if roots.iter().any(|&i| i >= nodes.len()) {
            return None;
        }
        let pending: Vec<P> = roots.iter().map(|&i| schedule.node(i)).collect();
        let all = schedule.all(&pending);
        let done = schedule.pending();
        schedule.on(
            all,
            Job::AttachScene {
                scene: index,
                roots,
                done,
            },
        );
        scene_promises.push(done);
    }
    // `parse()` requests all scenes first, then all animations. An animation
    // channel outside the default scene still calls getDependency('node');
    // its cached node can attach a shared child after a scene callback. Its
    // sampler accessors also start FileLoader work in channel order.
    for animation in json["animations"].as_array().into_iter().flatten() {
        for channel in animation["channels"].as_array().into_iter().flatten() {
            let Some(node) = id(&channel["target"]["node"]) else {
                continue;
            };
            if node >= nodes.len() {
                return None;
            }
            schedule.node(node);
            let Some(sampler) = id(&channel["sampler"]).and_then(|i| animation["samplers"].get(i))
            else {
                continue;
            };
            let resolve = |v: &Value| {
                id(v).or_else(|| {
                    v.as_str()
                        .and_then(|name| id(&animation["parameters"][name]))
                })
            };
            if let Some(i) = resolve(&sampler["input"]) {
                schedule.accessor(i);
            }
            if let Some(i) = resolve(&sampler["output"]) {
                schedule.accessor(i);
            }
        }
    }
    let scenes_done = schedule.all(&scene_promises);
    schedule.run();
    if schedule.bad || !schedule.promises[scenes_done].settled {
        return None;
    }
    let default_roots = default_roots
        .into_iter()
        .filter(|&i| schedule.root_owner[i] == Some(scene))
        .collect();
    Some(Selection {
        parents: schedule.parents,
        default_roots,
    })
}
