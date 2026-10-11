//! SPIR-V rewrites between glslang and the back ends (plan section 11, step 4).
//!
//! * `remap_varyings` pins fragment inputs to the vertex stage's locations by
//!   name. GL links varyings by name; Vulkan and Metal link by location, and
//!   glslang's automatic locations follow declaration order, so a shader that
//!   declares only `v_st` would otherwise read `v_uv`.
//! * `split_combined_samplers` replaces every `sampler2D` (an `OpTypeSampledImage`
//!   variable) with a separate image and sampler variable, rewriting loads,
//!   function parameters, and call sites. naga's SPIR-V front end maps a
//!   combined type onto the bare image and loses the sampler.

use std::collections::HashMap;

use rspirv::dr::{Instruction, Module, Operand};
use rspirv::spirv::{Decoration, Op, StorageClass, Word};

/// Sampler variables are bound at `image binding + SAMPLER_BINDING_OFFSET`.
pub const SAMPLER_BINDING_OFFSET: u32 = 32;

#[derive(Debug, Clone)]
pub struct RewriteError(pub String);
impl std::fmt::Display for RewriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn load(words: &[u32]) -> Result<Module, RewriteError> {
    rspirv::dr::load_words(words).map_err(|e| RewriteError(format!("SPIR-V load: {e}")))
}

pub fn assemble(module: &Module) -> Vec<u32> {
    use rspirv::binary::Assemble;
    module.assemble()
}

fn fresh(module: &mut Module) -> Word {
    let header = module.header.as_mut().expect("module header");
    let id = header.bound;
    header.bound += 1;
    id
}

pub fn names(module: &Module) -> HashMap<Word, String> {
    let mut out = HashMap::new();
    for inst in &module.debug_names {
        if inst.class.opcode == Op::Name {
            if let (Some(Operand::IdRef(id)), Some(Operand::LiteralString(name))) = (inst.operands.first(), inst.operands.get(1)) {
                out.insert(*id, name.clone());
            }
        }
    }
    out
}

fn id_operand(inst: &Instruction, index: usize) -> Option<Word> {
    match inst.operands.get(index) {
        Some(Operand::IdRef(id)) => Some(*id),
        _ => None,
    }
}

fn decoration_value(module: &Module, target: Word, decoration: Decoration) -> Option<u32> {
    module.annotations.iter().find_map(|inst| {
        if inst.class.opcode != Op::Decorate || id_operand(inst, 0) != Some(target) {
            return None;
        }
        match (inst.operands.get(1), inst.operands.get(2)) {
            (Some(Operand::Decoration(d)), Some(Operand::LiteralBit32(v))) if *d == decoration => Some(*v),
            _ => None,
        }
    })
}

/// Pin `Input` variables named in `locations` to the given locations; other
/// inputs move past the largest pinned location so nothing collides.
pub fn remap_varyings(module: &mut Module, locations: &[(&str, u32)]) {
    let names = names(module);
    let inputs: Vec<Word> = module
        .types_global_values
        .iter()
        .filter(|inst| inst.class.opcode == Op::Variable && matches!(inst.operands.first(), Some(Operand::StorageClass(StorageClass::Input))))
        .filter_map(|inst| inst.result_id)
        .collect();
    let mut next = locations.iter().map(|(_, l)| l + 1).max().unwrap_or(0);
    for id in inputs {
        let wanted = names.get(&id).and_then(|name| locations.iter().find(|(n, _)| n == name).map(|(_, l)| *l));
        let has_location = decoration_value(module, id, Decoration::Location).is_some();
        if !has_location {
            continue; // built-ins (gl_FragCoord) carry BuiltIn, not Location
        }
        let location = wanted.unwrap_or_else(|| {
            let l = next;
            next += 1;
            l
        });
        for inst in module.annotations.iter_mut() {
            if inst.class.opcode == Op::Decorate && id_operand(inst, 0) == Some(id) {
                if let Some(Operand::Decoration(Decoration::Location)) = inst.operands.get(1) {
                    inst.operands[2] = Operand::LiteralBit32(location);
                }
            }
        }
    }
}

/// Give every resource of a stage a distinct binding in declaration order: the
/// default uniform block keeps `block_binding`, and each `UniformConstant`
/// variable (the `sampler2D`s) takes the next free number after it.
///
/// glslang's automatic bindings only cover live resources, so a declared but
/// unused `sampler2D` (legal in GL, where it is simply inactive) kept binding 0
/// and collided with the default block.
pub fn assign_bindings(module: &mut Module, block_binding: u32) {
    let mut next = block_binding + 1;
    let variables: Vec<(Word, StorageClass)> = module
        .types_global_values
        .iter()
        .filter(|inst| inst.class.opcode == Op::Variable)
        .filter_map(|inst| match inst.operands.first() {
            Some(Operand::StorageClass(class @ (StorageClass::Uniform | StorageClass::UniformConstant))) => Some((inst.result_id?, *class)),
            _ => None,
        })
        .collect();
    for (id, class) in variables {
        let binding = if class == StorageClass::Uniform {
            block_binding
        } else {
            next += 1;
            next - 1
        };
        let mut found = false;
        for inst in module.annotations.iter_mut() {
            if inst.class.opcode == Op::Decorate && id_operand(inst, 0) == Some(id) {
                if let Some(Operand::Decoration(Decoration::Binding)) = inst.operands.get(1) {
                    inst.operands[2] = Operand::LiteralBit32(binding);
                    found = true;
                }
            }
        }
        if !found {
            module.annotations.push(Instruction::new(Op::Decorate, None, None, vec![Operand::IdRef(id), Operand::Decoration(Decoration::Binding), Operand::LiteralBit32(binding)]));
        }
    }
}

/// One split resource: a former combined sampler variable or parameter.
#[derive(Clone, Copy, Debug)]
struct Pair {
    image: Word,
    sampler: Word,
}

#[derive(Debug, Clone)]
pub struct SplitBinding {
    pub name: String,
    pub set: u32,
    pub image_binding: u32,
    pub sampler_binding: u32,
}

/// Find a type instruction matching `pred`, or append one.
fn find_or_add_type(module: &mut Module, opcode: Op, operands: Vec<Operand>) -> Word {
    if let Some(existing) = module
        .types_global_values
        .iter()
        .find(|inst| inst.class.opcode == opcode && inst.operands == operands)
        .and_then(|inst| inst.result_id)
    {
        return existing;
    }
    let id = fresh(module);
    let position = after_dependencies(module, &operands);
    module.types_global_values.insert(position, Instruction::new(opcode, None, Some(id), operands));
    id
}

/// Index just after the last global instruction that defines an id referenced
/// by `operands`. glslang interleaves types with variables, so a new type must
/// go after its own operands, not merely before the first variable.
fn after_dependencies(module: &Module, operands: &[Operand]) -> usize {
    operands
        .iter()
        .filter_map(|op| match op {
            Operand::IdRef(id) => module.types_global_values.iter().position(|inst| inst.result_id == Some(*id)),
            _ => None,
        })
        .max()
        .map_or(0, |p| p + 1)
}

pub fn split_combined_samplers(module: &mut Module) -> Result<Vec<SplitBinding>, RewriteError> {
    let names = names(module);
    // sampled image type -> image type
    let sampled_images: HashMap<Word, Word> = module
        .types_global_values
        .iter()
        .filter(|inst| inst.class.opcode == Op::TypeSampledImage)
        .map(|inst| (inst.result_id.unwrap(), id_operand(inst, 0).unwrap()))
        .collect();
    if sampled_images.is_empty() {
        return Ok(Vec::new());
    }
    // pointer-to-sampled-image types (UniformConstant) -> sampled image type
    let mut combined_pointers: HashMap<Word, Word> = HashMap::new();
    for inst in &module.types_global_values {
        if inst.class.opcode == Op::TypePointer {
            if let (Some(Operand::StorageClass(StorageClass::UniformConstant)), Some(Operand::IdRef(pointee))) = (inst.operands.first(), inst.operands.get(1)) {
                if sampled_images.contains_key(pointee) {
                    combined_pointers.insert(inst.result_id.unwrap(), *pointee);
                }
            }
            if let (Some(_), Some(Operand::IdRef(pointee))) = (inst.operands.first(), inst.operands.get(1)) {
                let is_array_of_sampled = module.types_global_values.iter().any(|t| {
                    t.result_id == Some(*pointee)
                        && matches!(t.class.opcode, Op::TypeArray | Op::TypeRuntimeArray)
                        && id_operand(t, 0).is_some_and(|e| sampled_images.contains_key(&e))
                });
                if is_array_of_sampled {
                    return Err(RewriteError("arrays of sampler2D are not supported by the sampler split".into()));
                }
            }
        }
    }
    let sampler_type = find_or_add_type(module, Op::TypeSampler, vec![]);
    let sampler_pointer = find_or_add_type(module, Op::TypePointer, vec![Operand::StorageClass(StorageClass::UniformConstant), Operand::IdRef(sampler_type)]);
    let mut image_pointer_for: HashMap<Word, Word> = HashMap::new(); // combined pointer type -> image pointer type
    for (&pointer, &sampled) in &combined_pointers {
        let image = sampled_images[&sampled];
        let id = find_or_add_type(module, Op::TypePointer, vec![Operand::StorageClass(StorageClass::UniformConstant), Operand::IdRef(image)]);
        image_pointer_for.insert(pointer, id);
    }

    // 1. Global variables.
    let mut globals: HashMap<Word, Pair> = HashMap::new();
    let mut bindings = Vec::new();
    let combined_vars: Vec<(Word, Word)> = module
        .types_global_values
        .iter()
        .filter(|inst| inst.class.opcode == Op::Variable && inst.result_type.is_some_and(|t| combined_pointers.contains_key(&t)))
        .map(|inst| (inst.result_id.unwrap(), inst.result_type.unwrap()))
        .collect();
    for (var, pointer) in combined_vars {
        let set = decoration_value(module, var, Decoration::DescriptorSet).unwrap_or(0);
        let binding = decoration_value(module, var, Decoration::Binding)
            .ok_or_else(|| RewriteError(format!("sampler variable %{var} has no binding")))?;
        let image_var = fresh(module);
        let sampler_var = fresh(module);
        let position = module.types_global_values.iter().position(|inst| inst.result_id == Some(var)).unwrap();
        module.types_global_values[position] = Instruction::new(Op::Variable, Some(image_pointer_for[&pointer]), Some(image_var), vec![Operand::StorageClass(StorageClass::UniformConstant)]);
        module.types_global_values.insert(position + 1, Instruction::new(Op::Variable, Some(sampler_pointer), Some(sampler_var), vec![Operand::StorageClass(StorageClass::UniformConstant)]));
        module.annotations.retain(|inst| !(matches!(inst.class.opcode, Op::Decorate) && id_operand(inst, 0) == Some(var)));
        for (target, b) in [(image_var, binding), (sampler_var, binding + SAMPLER_BINDING_OFFSET)] {
            module.annotations.push(Instruction::new(Op::Decorate, None, None, vec![Operand::IdRef(target), Operand::Decoration(Decoration::DescriptorSet), Operand::LiteralBit32(set)]));
            module.annotations.push(Instruction::new(Op::Decorate, None, None, vec![Operand::IdRef(target), Operand::Decoration(Decoration::Binding), Operand::LiteralBit32(b)]));
        }
        let name = names.get(&var).cloned().unwrap_or_else(|| format!("sampler{var}"));
        module.debug_names.retain(|inst| !(inst.class.opcode == Op::Name && id_operand(inst, 0) == Some(var)));
        module.debug_names.push(Instruction::new(Op::Name, None, None, vec![Operand::IdRef(image_var), Operand::LiteralString(name.clone())]));
        module.debug_names.push(Instruction::new(Op::Name, None, None, vec![Operand::IdRef(sampler_var), Operand::LiteralString(format!("{name}_sampler"))]));
        for entry in module.entry_points.iter_mut() {
            entry.operands.retain(|op| *op != Operand::IdRef(var));
        }
        globals.insert(var, Pair { image: image_var, sampler: sampler_var });
        bindings.push(SplitBinding { name, set, image_binding: binding, sampler_binding: binding + SAMPLER_BINDING_OFFSET });
    }

    // 2. Function signatures: split combined-pointer parameters.
    let mut function_types: HashMap<Word, Word> = HashMap::new(); // function id -> new type
    let mut params: HashMap<Word, Pair> = HashMap::new();
    let function_count = module.functions.len();
    for index in 0..function_count {
        let has_combined = module.functions[index].parameters.iter().any(|p| p.result_type.is_some_and(|t| combined_pointers.contains_key(&t)));
        if !has_combined {
            continue;
        }
        let def = module.functions[index].def.clone().unwrap();
        let return_type = def.result_type.unwrap();
        let mut new_params = Vec::new();
        let mut type_operands = vec![Operand::IdRef(return_type)];
        let old_params = module.functions[index].parameters.clone();
        for param in old_params {
            let ty = param.result_type.unwrap();
            if let Some(&image_pointer) = image_pointer_for.get(&ty) {
                let image = fresh(module);
                let sampler = fresh(module);
                new_params.push(Instruction::new(Op::FunctionParameter, Some(image_pointer), Some(image), vec![]));
                new_params.push(Instruction::new(Op::FunctionParameter, Some(sampler_pointer), Some(sampler), vec![]));
                type_operands.push(Operand::IdRef(image_pointer));
                type_operands.push(Operand::IdRef(sampler_pointer));
                params.insert(param.result_id.unwrap(), Pair { image, sampler });
            } else {
                type_operands.push(Operand::IdRef(ty));
                new_params.push(param);
            }
        }
        // OpTypeFunction must be unique: reuse an identical signature.
        let new_type = {
            let existing = module
                .types_global_values
                .iter()
                .find(|inst| inst.class.opcode == Op::TypeFunction && inst.operands == type_operands)
                .and_then(|inst| inst.result_id);
            match existing {
                Some(id) => id,
                None => {
                    let id = fresh(module);
                    let position = after_dependencies(module, &type_operands);
                    module.types_global_values.insert(position, Instruction::new(Op::TypeFunction, None, Some(id), type_operands.clone()));
                    id
                }
            }
        };
        let function = &mut module.functions[index];
        function.parameters = new_params;
        let def = function.def.as_mut().unwrap();
        def.operands[1] = Operand::IdRef(new_type);
        function_types.insert(def.result_id.unwrap(), new_type);
    }

    // 3. Loads and call sites.
    let lookup = |id: Word| globals.get(&id).or_else(|| params.get(&id)).copied();
    let mut pending: Vec<(usize, usize, usize, Vec<Instruction>)> = Vec::new();
    let mut fresh_ids: Vec<Word> = Vec::new();
    // Reserve ids lazily: count loads first.
    for (fi, function) in module.functions.iter().enumerate() {
        for (bi, block) in function.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                match inst.class.opcode {
                    Op::Load => {
                        let pointer = id_operand(inst, 0).unwrap();
                        if let Some(pair) = lookup(pointer) {
                            pending.push((fi, bi, ii, vec![inst.clone(), Instruction::new(Op::Nop, None, None, vec![Operand::IdRef(pair.image), Operand::IdRef(pair.sampler)])]));
                        }
                    }
                    Op::FunctionCall => {
                        let mut operands = Vec::new();
                        let mut changed = false;
                        for (k, op) in inst.operands.iter().enumerate() {
                            if k > 0 {
                                if let Operand::IdRef(arg) = op {
                                    if let Some(pair) = lookup(*arg) {
                                        operands.push(Operand::IdRef(pair.image));
                                        operands.push(Operand::IdRef(pair.sampler));
                                        changed = true;
                                        continue;
                                    }
                                }
                            }
                            operands.push(op.clone());
                        }
                        if changed {
                            pending.push((fi, bi, ii, vec![Instruction::new(Op::FunctionCall, inst.result_type, inst.result_id, operands)]));
                        }
                    }
                    Op::CopyObject | Op::Select | Op::Phi | Op::Store => {
                        if inst.operands.iter().any(|op| matches!(op, Operand::IdRef(id) if lookup(*id).is_some())) {
                            return Err(RewriteError(format!("unsupported use of a sampler2D in {:?}", inst.class.opcode)));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for _ in 0..pending.iter().filter(|p| p.3.len() == 2).count() * 2 {
        fresh_ids.push(fresh(module));
    }
    let mut ids = fresh_ids.into_iter();
    // Apply from the back so indices stay valid.
    for (fi, bi, ii, replacement) in pending.into_iter().rev() {
        let block = &mut module.functions[fi].blocks[bi];
        if replacement.len() == 2 {
            let load = &replacement[0];
            let (image, sampler) = match (replacement[1].operands.first(), replacement[1].operands.get(1)) {
                (Some(Operand::IdRef(i)), Some(Operand::IdRef(s))) => (*i, *s),
                _ => unreachable!(),
            };
            let sampled_type = load.result_type.unwrap();
            let image_type = sampled_images
                .get(&sampled_type)
                .copied()
                .ok_or_else(|| RewriteError("load of a combined sampler with an unexpected type".into()))?;
            let image_value = ids.next().unwrap();
            let sampler_value = ids.next().unwrap();
            let new = vec![
                Instruction::new(Op::Load, Some(image_type), Some(image_value), vec![Operand::IdRef(image)]),
                Instruction::new(Op::Load, Some(sampler_type), Some(sampler_value), vec![Operand::IdRef(sampler)]),
                Instruction::new(Op::SampledImage, Some(sampled_type), load.result_id, vec![Operand::IdRef(image_value), Operand::IdRef(sampler_value)]),
            ];
            block.instructions.splice(ii..=ii, new);
        } else {
            block.instructions[ii] = replacement.into_iter().next().unwrap();
        }
    }
    // Drop now-unused combined pointer types only if nothing references them.
    let _ = function_types;
    Ok(bindings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{Stage, compile};

    const HELPER: &str = "#version 450\nlayout(location=0) in vec2 v_st;\nuniform sampler2D u_tex; uniform sampler2D u_orig;\nout vec4 o;\nvec4 tap(sampler2D s, vec2 uv){ return texture(s, uv) * float(textureSize(s, 0).x > 0); }\nvec4 twice(sampler2D a, sampler2D b, vec2 uv){ return tap(a, uv) + tap(b, uv); }\nvoid main(){ o = twice(u_tex, u_orig, v_st) + texelFetch(u_tex, ivec2(0), 0); }\n";

    #[test]
    fn splits_globals_parameters_and_calls() {
        let spv = compile(HELPER, Stage::Fragment).unwrap();
        let mut module = load(&spv.words).unwrap();
        let bindings = split_combined_samplers(&mut module).unwrap();
        assert_eq!(bindings.len(), 2);
        let words = assemble(&module);
        let parsed = naga::front::spv::Frontend::new(words.into_iter(), &naga::front::spv::Options::default()).parse().expect("naga parses the split module");
        let samplers = parsed.global_variables.iter().filter(|(_, v)| matches!(parsed.types[v.ty].inner, naga::TypeInner::Sampler { .. })).count();
        assert_eq!(samplers, 2);
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&parsed).expect("valid");
    }

    #[test]
    fn new_types_follow_their_operands() {
        // The built-in crossfade: glslang emits the `o` output variable before
        // the image type, so a pointer-to-image inserted "before the first
        // variable" referenced an undefined id (naga: InvalidId(10)).
        let src = "#version 450\nin vec2 v_uv; in vec2 v_st;\nuniform sampler2D u_from, u_to;\nuniform float u_prog;\nout vec4 o;\nvoid main(){ o = mix(texture(u_from, v_st), texture(u_to, v_st), clamp(u_prog, 0.0, 1.0)); }\n";
        let spv = compile(src, Stage::Fragment).unwrap();
        let mut module = load(&spv.words).unwrap();
        split_combined_samplers(&mut module).unwrap();
        let words = assemble(&module);
        naga::front::spv::Frontend::new(words.into_iter(), &naga::front::spv::Options::default()).parse().expect("types precede uses");
    }

    #[test]
    fn unused_samplers_get_their_own_binding() {
        let src = "#version 450\nuniform sampler2D u_tex; uniform sampler2D u_orig; uniform float u_time;\nout vec4 o;\nvoid main(){ o = vec4(u_time) + texture(u_orig, vec2(0.5)); }\n";
        let spv = compile(src, Stage::Fragment).unwrap();
        let mut module = load(&spv.words).unwrap();
        assign_bindings(&mut module, 0);
        let binding_of = |name: &str| {
            let id = names(&module).into_iter().find(|(_, n)| n == name).map(|(id, _)| id).unwrap();
            decoration_value(&module, id, Decoration::Binding).unwrap()
        };
        let block = module
            .types_global_values
            .iter()
            .find(|inst| inst.class.opcode == Op::Variable && matches!(inst.operands.first(), Some(Operand::StorageClass(StorageClass::Uniform))))
            .and_then(|inst| inst.result_id)
            .unwrap();
        assert_eq!(decoration_value(&module, block, Decoration::Binding), Some(0));
        // SPIR-V order is glslang's emission order (live resources first).
        let mut samplers = [binding_of("u_tex"), binding_of("u_orig")];
        samplers.sort();
        assert_eq!(samplers, [1, 2], "the unused sampler no longer shares binding 0 with the block");
    }

    #[test]
    fn varyings_follow_names_not_declaration_order() {
        let spv = compile("#version 450\nin vec2 v_px; in vec2 v_st; out vec4 o; void main(){ o = vec4(v_px, v_st); }\n", Stage::Fragment).unwrap();
        let mut module = load(&spv.words).unwrap();
        remap_varyings(&mut module, &[("v_uv", 0), ("v_st", 1), ("v_px", 2)]);
        let names = names(&module);
        for (id, name) in names {
            if name == "v_st" {
                assert_eq!(decoration_value(&module, id, Decoration::Location), Some(1));
            }
            if name == "v_px" {
                assert_eq!(decoration_value(&module, id, Decoration::Location), Some(2));
            }
        }
    }
}
