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

use std::collections::{HashMap, HashSet};

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
        if inst.class.opcode == Op::Name
            && let (Some(Operand::IdRef(id)), Some(Operand::LiteralString(name))) =
                (inst.operands.first(), inst.operands.get(1))
        {
            out.insert(*id, name.clone());
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
            (Some(Operand::Decoration(d)), Some(Operand::LiteralBit32(v))) if *d == decoration => {
                Some(*v)
            }
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
        .filter(|inst| {
            inst.class.opcode == Op::Variable
                && matches!(
                    inst.operands.first(),
                    Some(Operand::StorageClass(StorageClass::Input))
                )
        })
        .filter_map(|inst| inst.result_id)
        .collect();
    let mut next = locations.iter().map(|(_, l)| l + 1).max().unwrap_or(0);
    for id in inputs {
        let wanted = names
            .get(&id)
            .and_then(|name| locations.iter().find(|(n, _)| n == name).map(|(_, l)| *l));
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
            if inst.class.opcode == Op::Decorate
                && id_operand(inst, 0) == Some(id)
                && let Some(Operand::Decoration(Decoration::Location)) = inst.operands.get(1)
            {
                inst.operands[2] = Operand::LiteralBit32(location);
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
            Some(Operand::StorageClass(
                class @ (StorageClass::Uniform | StorageClass::UniformConstant),
            )) => Some((inst.result_id?, *class)),
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
            if inst.class.opcode == Op::Decorate
                && id_operand(inst, 0) == Some(id)
                && let Some(Operand::Decoration(Decoration::Binding)) = inst.operands.get(1)
            {
                inst.operands[2] = Operand::LiteralBit32(binding);
                found = true;
            }
        }
        if !found {
            module.annotations.push(Instruction::new(
                Op::Decorate,
                None,
                None,
                vec![
                    Operand::IdRef(id),
                    Operand::Decoration(Decoration::Binding),
                    Operand::LiteralBit32(binding),
                ],
            ));
        }
    }
}

/// One split resource: a former combined sampler variable or parameter.
#[derive(Clone, Copy, Debug)]
struct Pair {
    image: Word,
    sampler: Word,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    module
        .types_global_values
        .insert(position, Instruction::new(opcode, None, Some(id), operands));
    id
}

/// Index just after the last global instruction that defines an id referenced
/// by `operands`. glslang interleaves types with variables, so a new type must
/// go after its own operands, not merely before the first variable.
fn after_dependencies(module: &Module, operands: &[Operand]) -> usize {
    operands
        .iter()
        .filter_map(|op| match op {
            Operand::IdRef(id) => module
                .types_global_values
                .iter()
                .position(|inst| inst.result_id == Some(*id)),
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
            if let (
                Some(Operand::StorageClass(StorageClass::UniformConstant)),
                Some(Operand::IdRef(pointee)),
            ) = (inst.operands.first(), inst.operands.get(1))
                && sampled_images.contains_key(pointee)
            {
                combined_pointers.insert(inst.result_id.unwrap(), *pointee);
            }
            // Arrays are scalarized before this (`scalarize_sampler_arrays`);
            // one left on a variable means that rewrite was skipped.
            let pointer_used = module
                .types_global_values
                .iter()
                .any(|v| v.class.opcode == Op::Variable && v.result_type == inst.result_id);
            if let (true, Some(_), Some(Operand::IdRef(pointee))) =
                (pointer_used, inst.operands.first(), inst.operands.get(1))
            {
                let is_array_of_sampled = module.types_global_values.iter().any(|t| {
                    t.result_id == Some(*pointee)
                        && matches!(t.class.opcode, Op::TypeArray | Op::TypeRuntimeArray)
                        && id_operand(t, 0).is_some_and(|e| sampled_images.contains_key(&e))
                });
                if is_array_of_sampled {
                    return Err(RewriteError(
                        "arrays of sampler2D are not supported by the sampler split".into(),
                    ));
                }
            }
        }
    }
    let sampler_type = find_or_add_type(module, Op::TypeSampler, vec![]);
    let sampler_pointer = find_or_add_type(
        module,
        Op::TypePointer,
        vec![
            Operand::StorageClass(StorageClass::UniformConstant),
            Operand::IdRef(sampler_type),
        ],
    );
    let mut image_pointer_for: HashMap<Word, Word> = HashMap::new(); // combined pointer type -> image pointer type
    for (&pointer, &sampled) in &combined_pointers {
        let image = sampled_images[&sampled];
        let id = find_or_add_type(
            module,
            Op::TypePointer,
            vec![
                Operand::StorageClass(StorageClass::UniformConstant),
                Operand::IdRef(image),
            ],
        );
        image_pointer_for.insert(pointer, id);
    }

    // 1. Global variables.
    let mut globals: HashMap<Word, Pair> = HashMap::new();
    let mut bindings = Vec::new();
    let combined_vars: Vec<(Word, Word)> = module
        .types_global_values
        .iter()
        .filter(|inst| {
            inst.class.opcode == Op::Variable
                && inst
                    .result_type
                    .is_some_and(|t| combined_pointers.contains_key(&t))
        })
        .map(|inst| (inst.result_id.unwrap(), inst.result_type.unwrap()))
        .collect();
    for (var, pointer) in combined_vars {
        let set = decoration_value(module, var, Decoration::DescriptorSet).unwrap_or(0);
        let binding = decoration_value(module, var, Decoration::Binding)
            .ok_or_else(|| RewriteError(format!("sampler variable %{var} has no binding")))?;
        let image_var = fresh(module);
        let sampler_var = fresh(module);
        let position = module
            .types_global_values
            .iter()
            .position(|inst| inst.result_id == Some(var))
            .unwrap();
        module.types_global_values[position] = Instruction::new(
            Op::Variable,
            Some(image_pointer_for[&pointer]),
            Some(image_var),
            vec![Operand::StorageClass(StorageClass::UniformConstant)],
        );
        module.types_global_values.insert(
            position + 1,
            Instruction::new(
                Op::Variable,
                Some(sampler_pointer),
                Some(sampler_var),
                vec![Operand::StorageClass(StorageClass::UniformConstant)],
            ),
        );
        module.annotations.retain(|inst| {
            !(matches!(inst.class.opcode, Op::Decorate) && id_operand(inst, 0) == Some(var))
        });
        for (target, b) in [
            (image_var, binding),
            (sampler_var, binding + SAMPLER_BINDING_OFFSET),
        ] {
            module.annotations.push(Instruction::new(
                Op::Decorate,
                None,
                None,
                vec![
                    Operand::IdRef(target),
                    Operand::Decoration(Decoration::DescriptorSet),
                    Operand::LiteralBit32(set),
                ],
            ));
            module.annotations.push(Instruction::new(
                Op::Decorate,
                None,
                None,
                vec![
                    Operand::IdRef(target),
                    Operand::Decoration(Decoration::Binding),
                    Operand::LiteralBit32(b),
                ],
            ));
        }
        let name = names
            .get(&var)
            .cloned()
            .unwrap_or_else(|| format!("sampler{var}"));
        module
            .debug_names
            .retain(|inst| !(inst.class.opcode == Op::Name && id_operand(inst, 0) == Some(var)));
        module.debug_names.push(Instruction::new(
            Op::Name,
            None,
            None,
            vec![
                Operand::IdRef(image_var),
                Operand::LiteralString(name.clone()),
            ],
        ));
        module.debug_names.push(Instruction::new(
            Op::Name,
            None,
            None,
            vec![
                Operand::IdRef(sampler_var),
                Operand::LiteralString(format!("{name}_sampler")),
            ],
        ));
        for entry in module.entry_points.iter_mut() {
            entry.operands.retain(|op| *op != Operand::IdRef(var));
        }
        globals.insert(
            var,
            Pair {
                image: image_var,
                sampler: sampler_var,
            },
        );
        bindings.push(SplitBinding {
            name,
            set,
            image_binding: binding,
            sampler_binding: binding + SAMPLER_BINDING_OFFSET,
        });
    }

    // 2. Function signatures: split combined-pointer parameters.
    let mut function_types: HashMap<Word, Word> = HashMap::new(); // function id -> new type
    let mut params: HashMap<Word, Pair> = HashMap::new();
    let function_count = module.functions.len();
    for index in 0..function_count {
        let has_combined = module.functions[index].parameters.iter().any(|p| {
            p.result_type
                .is_some_and(|t| combined_pointers.contains_key(&t))
        });
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
                new_params.push(Instruction::new(
                    Op::FunctionParameter,
                    Some(image_pointer),
                    Some(image),
                    vec![],
                ));
                new_params.push(Instruction::new(
                    Op::FunctionParameter,
                    Some(sampler_pointer),
                    Some(sampler),
                    vec![],
                ));
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
                .find(|inst| {
                    inst.class.opcode == Op::TypeFunction && inst.operands == type_operands
                })
                .and_then(|inst| inst.result_id);
            match existing {
                Some(id) => id,
                None => {
                    let id = fresh(module);
                    let position = after_dependencies(module, &type_operands);
                    module.types_global_values.insert(
                        position,
                        Instruction::new(Op::TypeFunction, None, Some(id), type_operands.clone()),
                    );
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
                            pending.push((
                                fi,
                                bi,
                                ii,
                                vec![
                                    inst.clone(),
                                    Instruction::new(
                                        Op::Nop,
                                        None,
                                        None,
                                        vec![
                                            Operand::IdRef(pair.image),
                                            Operand::IdRef(pair.sampler),
                                        ],
                                    ),
                                ],
                            ));
                        }
                    }
                    Op::FunctionCall => {
                        let mut operands = Vec::new();
                        let mut changed = false;
                        for (k, op) in inst.operands.iter().enumerate() {
                            if k > 0
                                && let Operand::IdRef(arg) = op
                                && let Some(pair) = lookup(*arg)
                            {
                                operands.push(Operand::IdRef(pair.image));
                                operands.push(Operand::IdRef(pair.sampler));
                                changed = true;
                                continue;
                            }
                            operands.push(op.clone());
                        }
                        if changed {
                            pending.push((
                                fi,
                                bi,
                                ii,
                                vec![Instruction::new(
                                    Op::FunctionCall,
                                    inst.result_type,
                                    inst.result_id,
                                    operands,
                                )],
                            ));
                        }
                    }
                    Op::CopyObject | Op::Select | Op::Phi | Op::Store
                        if inst.operands.iter().any(
                            |op| matches!(op, Operand::IdRef(id) if lookup(*id).is_some()),
                        ) =>
                    {
                        return Err(RewriteError(format!(
                            "unsupported use of a sampler2D in {:?}",
                            inst.class.opcode
                        )));
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
            let (image, sampler) = match (
                replacement[1].operands.first(),
                replacement[1].operands.get(1),
            ) {
                (Some(Operand::IdRef(i)), Some(Operand::IdRef(s))) => (*i, *s),
                _ => unreachable!(),
            };
            let sampled_type = load.result_type.unwrap();
            let image_type = sampled_images.get(&sampled_type).copied().ok_or_else(|| {
                RewriteError("load of a combined sampler with an unexpected type".into())
            })?;
            let image_value = ids.next().unwrap();
            let sampler_value = ids.next().unwrap();
            let new = vec![
                Instruction::new(
                    Op::Load,
                    Some(image_type),
                    Some(image_value),
                    vec![Operand::IdRef(image)],
                ),
                Instruction::new(
                    Op::Load,
                    Some(sampler_type),
                    Some(sampler_value),
                    vec![Operand::IdRef(sampler)],
                ),
                Instruction::new(
                    Op::SampledImage,
                    Some(sampled_type),
                    load.result_id,
                    vec![Operand::IdRef(image_value), Operand::IdRef(sampler_value)],
                ),
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

/// naga's validator rejects non-finite float literals in constant
/// expressions (`ConstExpression { source: Literal(Infinity) }`), while WebGL
/// accepts them: `1e39` and `1.0 / 0.0` fold to infinity in ANGLE and in
/// glslang. Every non-finite `OpConstant` (and every constant composite built
/// from one) is rebuilt at run time in the functions that use it, from the
/// same bits through `OpBitcast` of a `uint` constant, which naga does not
/// fold. Private variable initializers that use one become stores at the
/// start of the entry point. Returns the number of constants rewritten.
pub fn materialize_non_finite_constants(module: &mut Module) -> usize {
    let float_types: HashSet<Word> = module
        .types_global_values
        .iter()
        .filter(|i| {
            i.class.opcode == Op::TypeFloat
                && matches!(i.operands.first(), Some(Operand::LiteralBit32(32)))
        })
        .filter_map(|i| i.result_id)
        .collect();
    let mut tainted: Vec<Word> = Vec::new();
    let mut tainted_set: HashSet<Word> = HashSet::new();
    let mut bits_of: HashMap<Word, u32> = HashMap::new();
    for inst in &module.types_global_values {
        let Some(id) = inst.result_id else { continue };
        let hit = match inst.class.opcode {
            Op::Constant if inst.result_type.is_some_and(|t| float_types.contains(&t)) => {
                match inst.operands.first() {
                    Some(Operand::LiteralBit32(bits)) if !f32::from_bits(*bits).is_finite() => {
                        bits_of.insert(id, *bits);
                        true
                    }
                    _ => false,
                }
            }
            Op::ConstantComposite => inst
                .operands
                .iter()
                .any(|o| matches!(o, Operand::IdRef(r) if tainted_set.contains(r))),
            _ => false,
        };
        if hit {
            tainted.push(id);
            tainted_set.insert(id);
        }
    }
    if tainted.is_empty() {
        return 0;
    }
    let uint = find_or_add_type(
        module,
        Op::TypeInt,
        vec![Operand::LiteralBit32(32), Operand::LiteralBit32(0)],
    );
    let mut uint_constant: HashMap<u32, Word> = HashMap::new();
    for bits in bits_of.values() {
        if uint_constant.contains_key(bits) {
            continue;
        }
        let existing = module
            .types_global_values
            .iter()
            .find(|i| {
                i.class.opcode == Op::Constant
                    && i.result_type == Some(uint)
                    && i.operands == [Operand::LiteralBit32(*bits)]
            })
            .and_then(|i| i.result_id);
        let id = existing.unwrap_or_else(|| {
            let id = fresh(module);
            let position = after_dependencies(module, &[Operand::IdRef(uint)]);
            module.types_global_values.insert(
                position,
                Instruction::new(
                    Op::Constant,
                    Some(uint),
                    Some(id),
                    vec![Operand::LiteralBit32(*bits)],
                ),
            );
            id
        });
        uint_constant.insert(*bits, id);
    }
    let definitions: HashMap<Word, Instruction> = module
        .types_global_values
        .iter()
        .filter(|i| i.result_id.is_some_and(|id| tainted_set.contains(&id)))
        .map(|i| (i.result_id.unwrap(), i.clone()))
        .collect();
    // Private variable initializers become stores in the entry point.
    let mut global_stores: Vec<(Word, Word)> = Vec::new();
    for inst in module.types_global_values.iter_mut() {
        if inst.class.opcode == Op::Variable
            && let Some(Operand::IdRef(init)) = inst.operands.get(1)
            && tainted_set.contains(init)
        {
            global_stores.push((inst.result_id.unwrap(), *init));
            inst.operands.truncate(1);
        }
    }
    let entry = module.entry_points.first().and_then(|e| id_operand(e, 1));
    for index in 0..module.functions.len() {
        let is_entry = module.functions[index]
            .def
            .as_ref()
            .and_then(|d| d.result_id)
            == entry;
        // Tainted ids this function needs, including composite operands.
        let mut needed: HashSet<Word> = HashSet::new();
        let mut stack: Vec<Word> = Vec::new();
        for block in &module.functions[index].blocks {
            for inst in &block.instructions {
                for op in &inst.operands {
                    if let Operand::IdRef(r) = op
                        && tainted_set.contains(r)
                    {
                        stack.push(*r);
                    }
                }
            }
        }
        if is_entry {
            stack.extend(global_stores.iter().map(|(_, v)| *v));
        }
        while let Some(id) = stack.pop() {
            if needed.insert(id) {
                for op in &definitions[&id].operands {
                    if let Operand::IdRef(r) = op
                        && tainted_set.contains(r)
                    {
                        stack.push(*r);
                    }
                }
            }
        }
        if needed.is_empty() {
            continue;
        }
        let mut map: HashMap<Word, Word> = HashMap::new();
        let mut prologue = Vec::new();
        for id in tainted.iter().filter(|id| needed.contains(id)) {
            let def = &definitions[id];
            let new = fresh(module);
            map.insert(*id, new);
            if let Some(bits) = bits_of.get(id) {
                prologue.push(Instruction::new(
                    Op::Bitcast,
                    def.result_type,
                    Some(new),
                    vec![Operand::IdRef(uint_constant[bits])],
                ));
            } else {
                let operands = def.operands.iter().map(|o| match o {
                    Operand::IdRef(r) => Operand::IdRef(*map.get(r).unwrap_or(r)),
                    other => other.clone(),
                });
                prologue.push(Instruction::new(
                    Op::CompositeConstruct,
                    def.result_type,
                    Some(new),
                    operands.collect(),
                ));
            }
        }
        let function = &mut module.functions[index];
        let mut stores = Vec::new();
        for block in function.blocks.iter_mut() {
            for inst in block.instructions.iter_mut() {
                // Function-local variable initializers become stores too.
                if inst.class.opcode == Op::Variable
                    && let Some(Operand::IdRef(init)) = inst.operands.get(1)
                    && let Some(new) = map.get(init)
                {
                    stores.push(Instruction::new(
                        Op::Store,
                        None,
                        None,
                        vec![
                            Operand::IdRef(inst.result_id.unwrap()),
                            Operand::IdRef(*new),
                        ],
                    ));
                    inst.operands.truncate(1);
                    continue;
                }
                for op in inst.operands.iter_mut() {
                    if let Operand::IdRef(r) = op
                        && let Some(new) = map.get(r)
                    {
                        *r = *new;
                    }
                }
            }
        }
        if is_entry {
            for (var, value) in &global_stores {
                stores.push(Instruction::new(
                    Op::Store,
                    None,
                    None,
                    vec![Operand::IdRef(*var), Operand::IdRef(map[value])],
                ));
            }
        }
        let first = &mut function.blocks[0];
        let at = first
            .instructions
            .iter()
            .position(|i| i.class.opcode != Op::Variable)
            .unwrap_or(first.instructions.len());
        first
            .instructions
            .splice(at..at, prologue.into_iter().chain(stores));
    }
    // The global constants are now unused; naga would still reject them.
    module
        .types_global_values
        .retain(|i| !i.result_id.is_some_and(|id| tainted_set.contains(&id)));
    module
        .debug_names
        .retain(|i| !id_operand(i, 0).is_some_and(|id| tainted_set.contains(&id)));
    module
        .annotations
        .retain(|i| !id_operand(i, 0).is_some_and(|id| tainted_set.contains(&id)));
    tainted.len()
}

/* ── GL semantics the back ends lack (coverage sweep findings) ────────── */

/// Result type of every id that has one (globals, parameters, instructions).
fn result_types(module: &Module) -> HashMap<Word, Word> {
    let mut out = HashMap::new();
    let mut put = |inst: &Instruction| {
        if let (Some(id), Some(ty)) = (inst.result_id, inst.result_type) {
            out.insert(id, ty);
        }
    };
    module.types_global_values.iter().for_each(&mut put);
    for function in &module.functions {
        function.parameters.iter().for_each(&mut put);
        function
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .for_each(&mut put);
    }
    out
}

fn definition(module: &Module, id: Word) -> Option<&Instruction> {
    module
        .types_global_values
        .iter()
        .find(|i| i.result_id == Some(id))
}

fn constant_u32(module: &Module, id: Word) -> Option<u32> {
    let inst = definition(module, id)?;
    match (inst.class.opcode, inst.operands.first()) {
        (Op::Constant, Some(Operand::LiteralBit32(v))) => Some(*v),
        _ => None,
    }
}

/// An `OpConstant`/`OpConstantNull`/`OpConstantComposite` of `ty`, reused or
/// appended after its operands.
fn find_or_add_constant(module: &mut Module, opcode: Op, ty: Word, operands: Vec<Operand>) -> Word {
    if let Some(id) = module
        .types_global_values
        .iter()
        .find(|i| i.class.opcode == opcode && i.result_type == Some(ty) && i.operands == operands)
        .and_then(|i| i.result_id)
    {
        return id;
    }
    let id = fresh(module);
    let mut deps = operands.clone();
    deps.push(Operand::IdRef(ty));
    let position = after_dependencies(module, &deps);
    module.types_global_values.insert(
        position,
        Instruction::new(opcode, Some(ty), Some(id), operands),
    );
    id
}

fn float_type(module: &mut Module) -> Word {
    find_or_add_type(module, Op::TypeFloat, vec![Operand::LiteralBit32(32)])
}

fn is_referenced_in_functions(module: &Module, id: Word) -> bool {
    module
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter().flat_map(|b| &b.instructions))
        .any(|inst| inst.operands.contains(&Operand::IdRef(id)))
}

fn entry_function(module: &Module) -> Option<Word> {
    module.entry_points.first().and_then(|e| id_operand(e, 1))
}

/// Arrays of `sampler2D` become one combined-sampler variable per element,
/// named `name[k]` as GL names them. GLSL ES 3.00 indexes sampler arrays with
/// constant expressions only, which glslang folds to `OpConstant`, so every
/// `OpAccessChain` on the array picks one element and is replaced by that
/// element's variable. The result is plain `sampler2D` globals, which the
/// sampler split, the reflection, and both back ends already handle (naga's
/// split refused arrays, SPIRV-Cross needed array bindings the layout does
/// not have). Runs before `assign_bindings`, so each element gets its own
/// binding. Returns the number of arrays scalarized.
pub fn scalarize_sampler_arrays(module: &mut Module) -> Result<usize, RewriteError> {
    let names = names(module);
    let mut arrays: Vec<(Word, Word, u32)> = Vec::new(); // (variable, element type, length)
    for inst in &module.types_global_values {
        if inst.class.opcode != Op::Variable
            || inst.operands.first() != Some(&Operand::StorageClass(StorageClass::UniformConstant))
        {
            continue;
        }
        let Some(pointee) = inst
            .result_type
            .and_then(|t| definition(module, t))
            .and_then(|p| id_operand(p, 1))
        else {
            continue;
        };
        let Some(array) = definition(module, pointee).filter(|a| a.class.opcode == Op::TypeArray)
        else {
            continue;
        };
        let element = id_operand(array, 0).unwrap();
        if definition(module, element).is_some_and(|e| e.class.opcode == Op::TypeSampledImage) {
            let length = id_operand(array, 1)
                .and_then(|l| constant_u32(module, l))
                .ok_or_else(|| RewriteError("sampler array without a constant length".into()))?;
            arrays.push((inst.result_id.unwrap(), element, length));
        }
    }
    for &(var, element, length) in &arrays {
        let pointer = find_or_add_type(
            module,
            Op::TypePointer,
            vec![
                Operand::StorageClass(StorageClass::UniformConstant),
                Operand::IdRef(element),
            ],
        );
        let set = decoration_value(module, var, Decoration::DescriptorSet);
        let base_name = names
            .get(&var)
            .cloned()
            .unwrap_or_else(|| format!("samplers{var}"));
        let elements: Vec<Word> = (0..length).map(|_| fresh(module)).collect();
        let position = module
            .types_global_values
            .iter()
            .position(|i| i.result_id == Some(var))
            .unwrap();
        for (k, &id) in elements.iter().enumerate() {
            module.types_global_values.insert(
                position + 1 + k,
                Instruction::new(
                    Op::Variable,
                    Some(pointer),
                    Some(id),
                    vec![Operand::StorageClass(StorageClass::UniformConstant)],
                ),
            );
            module.debug_names.push(Instruction::new(
                Op::Name,
                None,
                None,
                vec![
                    Operand::IdRef(id),
                    Operand::LiteralString(format!("{base_name}[{k}]")),
                ],
            ));
            if let Some(set) = set {
                module.annotations.push(Instruction::new(
                    Op::Decorate,
                    None,
                    None,
                    vec![
                        Operand::IdRef(id),
                        Operand::Decoration(Decoration::DescriptorSet),
                        Operand::LiteralBit32(set),
                    ],
                ));
            }
        }
        // Element access chains → element variables.
        let mut replace: HashMap<Word, Word> = HashMap::new();
        for function in &module.functions {
            for inst in function.blocks.iter().flat_map(|b| &b.instructions) {
                if !inst.operands.contains(&Operand::IdRef(var)) {
                    continue;
                }
                let chain = matches!(inst.class.opcode, Op::AccessChain | Op::InBoundsAccessChain)
                    && id_operand(inst, 0) == Some(var)
                    && inst.operands.len() == 2;
                let index = if chain {
                    id_operand(inst, 1).and_then(|i| constant_u32(module, i))
                } else {
                    None
                };
                match index {
                    Some(k) if k < length => {
                        replace.insert(inst.result_id.unwrap(), elements[k as usize]);
                    }
                    _ => {
                        return Err(RewriteError(format!(
                            "sampler array `{base_name}` is used as a whole or with a non-constant index ({:?})",
                            inst.class.opcode
                        )));
                    }
                }
            }
        }
        for function in module.functions.iter_mut() {
            for block in function.blocks.iter_mut() {
                block
                    .instructions
                    .retain(|i| !i.result_id.is_some_and(|id| replace.contains_key(&id)));
                for inst in block.instructions.iter_mut() {
                    for op in inst.operands.iter_mut() {
                        if let Operand::IdRef(r) = op
                            && let Some(new) = replace.get(r)
                        {
                            *r = *new;
                        }
                    }
                }
            }
        }
        let in_interface = module
            .entry_points
            .iter()
            .any(|e| e.operands.contains(&Operand::IdRef(var)));
        for entry in module.entry_points.iter_mut() {
            entry.operands.retain(|op| *op != Operand::IdRef(var));
            if in_interface {
                entry
                    .operands
                    .extend(elements.iter().map(|&e| Operand::IdRef(e)));
            }
        }
        module
            .types_global_values
            .retain(|i| i.result_id != Some(var));
        module.debug_names.retain(|i| id_operand(i, 0) != Some(var));
        module.annotations.retain(|i| id_operand(i, 0) != Some(var));
    }
    Ok(arrays.len())
}

/// Turn an interface variable (`Input` or `Output`) into a `Private` one,
/// including the pointer types of access chains derived from it.
fn make_private(module: &mut Module, var: Word) {
    let types = result_types(module);
    let pointee =
        |module: &Module, pointer: Word| definition(module, pointer).and_then(|p| id_operand(p, 1));
    // Derived pointers, in instruction order (a chain's base precedes it).
    let mut derived: HashSet<Word> = HashSet::from([var]);
    let mut chains: Vec<Word> = Vec::new();
    for inst in module
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter().flat_map(|b| &b.instructions))
    {
        if matches!(inst.class.opcode, Op::AccessChain | Op::InBoundsAccessChain)
            && id_operand(inst, 0).is_some_and(|b| derived.contains(&b))
        {
            derived.insert(inst.result_id.unwrap());
            chains.push(inst.result_id.unwrap());
        }
    }
    let mut new_type: HashMap<Word, Word> = HashMap::new();
    for id in std::iter::once(var).chain(chains.iter().copied()) {
        let old = types[&id];
        let target = pointee(module, old).unwrap();
        let private = find_or_add_type(
            module,
            Op::TypePointer,
            vec![
                Operand::StorageClass(StorageClass::Private),
                Operand::IdRef(target),
            ],
        );
        new_type.insert(id, private);
    }
    for inst in module.types_global_values.iter_mut() {
        if inst.result_id == Some(var) {
            inst.result_type = Some(new_type[&var]);
            inst.operands[0] = Operand::StorageClass(StorageClass::Private);
        }
    }
    for function in module.functions.iter_mut() {
        for inst in function
            .blocks
            .iter_mut()
            .flat_map(|b| b.instructions.iter_mut())
        {
            if let Some(t) = inst.result_id.and_then(|id| new_type.get(&id)) {
                inst.result_type = Some(*t);
            }
        }
    }
    module.annotations.retain(|i| id_operand(i, 0) != Some(var));
    for entry in module.entry_points.iter_mut() {
        entry.operands.retain(|op| *op != Operand::IdRef(var));
    }
}

/// What WebGL does with a fragment interface the engine's pipeline would
/// otherwise reject (coverage sweep, `variables/*`, `qualifiers/*`):
///
/// * An input the vertex shader does not write and the program does not use
///   (GL links it; glslang keeps it in the interface and Metal then refuses
///   the pipeline) becomes a private variable.
/// * `gl_FragDepth`: the compositor's targets have no depth attachment, so
///   WebGL discards the write; it becomes a private variable (wgpu refuses a
///   depth output without a depth target).
/// * A `float`, `vec2`, or `vec3` output, or an array of float outputs: GL
///   writes the missing channels as undefined, ANGLE on Metal writes 0
///   (measured: `out vec3` gives alpha 0). wgpu needs a `vec4` output for an
///   RGBA target, and naga cannot output arrays. The author's variable becomes
///   private and every `return` from `main` copies it to `vec4` outputs at the
///   same locations, padded with 0.
///
/// Integer outputs are left alone: WebGL refuses the draw into a float target,
/// and wgpu refuses the pipeline. Returns notes for the report.
pub fn fragment_interface(module: &mut Module, vertex_outputs: &[&str]) -> Vec<String> {
    let names = names(module);
    let mut notes = Vec::new();
    let globals: Vec<(Word, StorageClass, Word)> = module
        .types_global_values
        .iter()
        .filter(|i| i.class.opcode == Op::Variable)
        .filter_map(|i| match i.operands.first() {
            Some(Operand::StorageClass(c @ (StorageClass::Input | StorageClass::Output))) => {
                Some((i.result_id?, *c, i.result_type?))
            }
            _ => None,
        })
        .collect();
    let builtin = |module: &Module, id: Word| decoration_value_any(module, id, Decoration::BuiltIn);
    for &(var, class, pointer) in &globals {
        let name = names.get(&var).cloned().unwrap_or_default();
        if class == StorageClass::Input {
            if builtin(module, var).is_none()
                && !vertex_outputs.contains(&name.as_str())
                && !is_referenced_in_functions(module, var)
            {
                make_private(module, var);
                notes.push(format!(
                    "unused input `{name}` the vertex shader does not write"
                ));
            }
            continue;
        }
        if builtin(module, var) == Some(rspirv::spirv::BuiltIn::FragDepth as u32) {
            make_private(module, var);
            module.execution_modes.retain(|m| {
                !matches!(
                    m.operands.get(1),
                    Some(Operand::ExecutionMode(
                        rspirv::spirv::ExecutionMode::DepthReplacing
                            | rspirv::spirv::ExecutionMode::DepthGreater
                            | rspirv::spirv::ExecutionMode::DepthLess
                            | rspirv::spirv::ExecutionMode::DepthUnchanged
                    ))
                )
            });
            notes.push("gl_FragDepth (no depth target)".into());
            continue;
        }
        let Some(location) = decoration_value(module, var, Decoration::Location) else {
            continue;
        };
        let Some(pointee) = definition(module, pointer).and_then(|p| id_operand(p, 1)) else {
            continue;
        };
        // Element type and count: a float scalar or vector, or an array of them.
        let (element, count) =
            match definition(module, pointee).map(|d| (d.class.opcode, d.clone())) {
                Some((Op::TypeArray, d)) => (
                    id_operand(&d, 0).unwrap(),
                    id_operand(&d, 1)
                        .and_then(|l| constant_u32(module, l))
                        .unwrap_or(0),
                ),
                _ => (pointee, 0),
            };
        let float = float_type(module);
        let width = match definition(module, element).map(|d| (d.class.opcode, d.clone())) {
            Some((Op::TypeFloat, _)) => 1,
            Some((Op::TypeVector, d)) if id_operand(&d, 0) == Some(float) => {
                match d.operands.get(1) {
                    Some(Operand::LiteralBit32(n)) => *n,
                    _ => continue,
                }
            }
            _ => continue,
        };
        if width == 4 && count == 0 {
            continue;
        }
        let vec4 = find_or_add_type(
            module,
            Op::TypeVector,
            vec![Operand::IdRef(float), Operand::LiteralBit32(4)],
        );
        let out_pointer = find_or_add_type(
            module,
            Op::TypePointer,
            vec![
                Operand::StorageClass(StorageClass::Output),
                Operand::IdRef(vec4),
            ],
        );
        let element_pointer = find_or_add_type(
            module,
            Op::TypePointer,
            vec![
                Operand::StorageClass(StorageClass::Private),
                Operand::IdRef(element),
            ],
        );
        let zero =
            find_or_add_constant(module, Op::Constant, float, vec![Operand::LiteralBit32(0)]);
        let int = find_or_add_type(
            module,
            Op::TypeInt,
            vec![Operand::LiteralBit32(32), Operand::LiteralBit32(1)],
        );
        make_private(module, var);
        let slots: Vec<Option<u32>> = if count == 0 {
            vec![None]
        } else {
            (0..count).map(Some).collect()
        };
        let mut outputs = Vec::new();
        for (k, slot) in slots.iter().enumerate() {
            let id = fresh(module);
            let position = after_dependencies(module, &[Operand::IdRef(out_pointer)]).max(
                module
                    .types_global_values
                    .iter()
                    .position(|i| i.result_id == Some(var))
                    .unwrap()
                    + 1,
            );
            module.types_global_values.insert(
                position,
                Instruction::new(
                    Op::Variable,
                    Some(out_pointer),
                    Some(id),
                    vec![Operand::StorageClass(StorageClass::Output)],
                ),
            );
            module.annotations.push(Instruction::new(
                Op::Decorate,
                None,
                None,
                vec![
                    Operand::IdRef(id),
                    Operand::Decoration(Decoration::Location),
                    Operand::LiteralBit32(location + k as u32),
                ],
            ));
            module.debug_names.push(Instruction::new(
                Op::Name,
                None,
                None,
                vec![
                    Operand::IdRef(id),
                    Operand::LiteralString(format!("{name}_out{k}")),
                ],
            ));
            for entry in module.entry_points.iter_mut() {
                entry.operands.push(Operand::IdRef(id));
            }
            let index = slot.map(|k| {
                find_or_add_constant(module, Op::Constant, int, vec![Operand::LiteralBit32(k)])
            });
            outputs.push((id, index));
        }
        let entry = entry_function(module);
        let Some(fi) = module
            .functions
            .iter()
            .position(|f| f.def.as_ref().and_then(|d| d.result_id) == entry)
        else {
            continue;
        };
        let returns: Vec<usize> = module.functions[fi]
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.instructions
                    .last()
                    .is_some_and(|i| i.class.opcode == Op::Return)
            })
            .map(|(k, _)| k)
            .collect();
        for bi in returns {
            let mut copy = Vec::new();
            for &(output, index) in &outputs {
                let source = match index {
                    Some(index) => {
                        let id = fresh(module);
                        copy.push(Instruction::new(
                            Op::AccessChain,
                            Some(element_pointer),
                            Some(id),
                            vec![Operand::IdRef(var), Operand::IdRef(index)],
                        ));
                        id
                    }
                    None => var,
                };
                let value = fresh(module);
                copy.push(Instruction::new(
                    Op::Load,
                    Some(element),
                    Some(value),
                    vec![Operand::IdRef(source)],
                ));
                let widened = if width == 4 {
                    value
                } else {
                    let id = fresh(module);
                    let mut parts = vec![Operand::IdRef(value)];
                    parts.extend(std::iter::repeat_n(
                        Operand::IdRef(zero),
                        (4 - width) as usize,
                    ));
                    copy.push(Instruction::new(
                        Op::CompositeConstruct,
                        Some(vec4),
                        Some(id),
                        parts,
                    ));
                    id
                };
                copy.push(Instruction::new(
                    Op::Store,
                    None,
                    None,
                    vec![Operand::IdRef(output), Operand::IdRef(widened)],
                ));
            }
            let block = &mut module.functions[fi].blocks[bi];
            let at = block.instructions.len() - 1;
            block.instructions.splice(at..at, copy);
        }
        notes.push(format!("output `{name}` widened to vec4"));
    }
    notes
}

/// `decoration_value` for decorations whose value is an enum (`BuiltIn`).
fn decoration_value_any(module: &Module, target: Word, decoration: Decoration) -> Option<u32> {
    module.annotations.iter().find_map(|inst| {
        if inst.class.opcode != Op::Decorate
            || id_operand(inst, 0) != Some(target)
            || inst.operands.get(1) != Some(&Operand::Decoration(decoration))
        {
            return None;
        }
        match inst.operands.get(2) {
            Some(Operand::BuiltIn(b)) => Some(*b as u32),
            Some(Operand::LiteralBit32(v)) => Some(*v),
            _ => None,
        }
    })
}

/// GLSL's `%` on signed integers: glslang emits `OpSMod` (result takes the
/// divisor's sign). GLSL ES leaves negative operands undefined; ANGLE emits
/// MSL's `%`, which is `OpSRem` (the dividend's sign). Rewriting to `OpSRem`
/// matches WebGL on both paths, and naga's front end lowers `OpSMod` through
/// `floor`, which its validator then refuses on integers
/// (`BuiltinArgumentsInvalid("Floor")`; coverage sweep, `operators/*`).
pub fn signed_remainder(module: &mut Module) -> usize {
    let mut n = 0;
    for inst in module
        .functions
        .iter_mut()
        .flat_map(|f| f.blocks.iter_mut().flat_map(|b| b.instructions.iter_mut()))
    {
        if inst.class.opcode == Op::SMod {
            *inst = Instruction::new(
                Op::SRem,
                inst.result_type,
                inst.result_id,
                inst.operands.clone(),
            );
            n += 1;
        }
    }
    n
}

/// SPIR-V that is valid but outside what naga's front end or validator take
/// (naga path only; coverage sweep):
///
/// * `OpSNegate` on `uint` (GLSL `-u`): naga refuses `Negate` on unsigned
///   types; `0 - u` has the same wrapping result.
/// * `OpImageFetch` with `ConstOffset` (`texelFetchOffset`): naga's front end
///   drops the offset; it is added to the coordinate instead (the same texel,
///   since the fetch has no wrapping).
/// * `MatrixStride` 16 on matrices with 2-component columns (`mat2`,
///   `mat3x2`, `mat4x2` uniforms, std140): naga only takes the stride of its
///   own layout (8) and refuses the module (`UnsupportedMatrixStride`). The
///   stride becomes 8 and the member keeps its offset, so later members do not
///   move; the reflection reads the new stride when filling the block.
pub fn naga_compat(module: &mut Module) -> Vec<String> {
    let mut notes = Vec::new();
    let types = result_types(module);
    let unsigned = |module: &Module, ty: Word| {
        let scalar = match definition(module, ty) {
            Some(d) if d.class.opcode == Op::TypeVector => {
                id_operand(d, 0).and_then(|s| definition(module, s))
            }
            other => other,
        };
        scalar.is_some_and(|s| {
            s.class.opcode == Op::TypeInt && s.operands.get(1) == Some(&Operand::LiteralBit32(0))
        })
    };
    // OpSNegate on unsigned.
    let negates: Vec<Word> = module
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter().flat_map(|b| &b.instructions))
        .filter(|i| {
            i.class.opcode == Op::SNegate && i.result_type.is_some_and(|t| unsigned(module, t))
        })
        .map(|i| i.result_type.unwrap())
        .collect();
    let nulls: HashMap<Word, Word> = negates
        .iter()
        .map(|&t| (t, find_or_add_constant(module, Op::ConstantNull, t, vec![])))
        .collect();
    let mut negated = 0;
    for inst in module
        .functions
        .iter_mut()
        .flat_map(|f| f.blocks.iter_mut().flat_map(|b| b.instructions.iter_mut()))
    {
        if inst.class.opcode == Op::SNegate
            && let Some(&null) = inst.result_type.and_then(|t| nulls.get(&t))
        {
            let x = inst.operands[0].clone();
            *inst = Instruction::new(
                Op::ISub,
                inst.result_type,
                inst.result_id,
                vec![Operand::IdRef(null), x],
            );
            negated += 1;
        }
    }
    if negated > 0 {
        notes.push(format!("uint negation ({negated})"));
    }
    // OpImageFetch with ConstOffset.
    let mut fetches = 0;
    for fi in 0..module.functions.len() {
        for bi in 0..module.functions[fi].blocks.len() {
            let mut k = 0;
            while k < module.functions[fi].blocks[bi].instructions.len() {
                let inst = &module.functions[fi].blocks[bi].instructions[k];
                let Some(Operand::ImageOperands(mask)) = inst.operands.get(2).cloned() else {
                    k += 1;
                    continue;
                };
                use rspirv::spirv::ImageOperands as M;
                if inst.class.opcode != Op::ImageFetch || !mask.contains(M::CONST_OFFSET) {
                    k += 1;
                    continue;
                }
                // Ids after the mask come in bit order; Grad takes two.
                let mut at = 3;
                for (bit, ids) in [(M::BIAS, 1), (M::LOD, 1), (M::GRAD, 2)] {
                    if mask.contains(bit) {
                        at += ids;
                    }
                }
                let coordinate = id_operand(inst, 1).unwrap();
                let offset = id_operand(inst, at).unwrap();
                let coordinate_type = types[&coordinate];
                let mut fetch = inst.clone();
                fetch.operands.remove(at);
                fetch.operands[2] = Operand::ImageOperands(mask.difference(M::CONST_OFFSET));
                let sum = fresh(module);
                fetch.operands[1] = Operand::IdRef(sum);
                let add = Instruction::new(
                    Op::IAdd,
                    Some(coordinate_type),
                    Some(sum),
                    vec![Operand::IdRef(coordinate), Operand::IdRef(offset)],
                );
                let block = &mut module.functions[fi].blocks[bi];
                block.instructions.splice(k..=k, [add, fetch]);
                fetches += 1;
                k += 2;
            }
        }
    }
    if fetches > 0 {
        notes.push(format!("texelFetchOffset ({fetches})"));
    }
    // MatrixStride of 2-row matrices.
    let two_row_matrix = |module: &Module, mut ty: Word| {
        while let Some(d) = definition(module, ty).filter(|d| d.class.opcode == Op::TypeArray) {
            ty = id_operand(d, 0).unwrap();
        }
        definition(module, ty)
            .filter(|d| d.class.opcode == Op::TypeMatrix)
            .and_then(|d| id_operand(d, 0))
            .and_then(|c| definition(module, c))
            .is_some_and(|c| c.operands.get(1) == Some(&Operand::LiteralBit32(2)))
    };
    let member_type = |module: &Module, structure: Word, member: u32| {
        definition(module, structure).and_then(|d| id_operand(d, member as usize))
    };
    let targets: Vec<usize> = module
        .annotations
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            i.class.opcode == Op::MemberDecorate
                && i.operands.get(2) == Some(&Operand::Decoration(Decoration::MatrixStride))
                && i.operands.get(3) == Some(&Operand::LiteralBit32(16))
        })
        .filter(|(_, i)| match (id_operand(i, 0), i.operands.get(1)) {
            (Some(s), Some(Operand::LiteralBit32(m))) => {
                member_type(module, s, *m).is_some_and(|t| two_row_matrix(module, t))
            }
            _ => false,
        })
        .map(|(k, _)| k)
        .collect();
    for &k in &targets {
        module.annotations[k].operands[3] = Operand::LiteralBit32(8);
    }
    if !targets.is_empty() {
        notes.push(format!("2-row matrix stride ({})", targets.len()));
    }
    notes
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
        let parsed = naga::front::spv::Frontend::new(
            words.into_iter(),
            &naga::front::spv::Options::default(),
        )
        .parse()
        .expect("naga parses the split module");
        let samplers = parsed
            .global_variables
            .iter()
            .filter(|(_, v)| matches!(parsed.types[v.ty].inner, naga::TypeInner::Sampler { .. }))
            .count();
        assert_eq!(samplers, 2);
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&parsed)
        .expect("valid");
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
        naga::front::spv::Frontend::new(words.into_iter(), &naga::front::spv::Options::default())
            .parse()
            .expect("types precede uses");
    }

    #[test]
    fn unused_samplers_get_their_own_binding() {
        let src = "#version 450\nuniform sampler2D u_tex; uniform sampler2D u_orig; uniform float u_time;\nout vec4 o;\nvoid main(){ o = vec4(u_time) + texture(u_orig, vec2(0.5)); }\n";
        let spv = compile(src, Stage::Fragment).unwrap();
        let mut module = load(&spv.words).unwrap();
        assign_bindings(&mut module, 0);
        let binding_of = |name: &str| {
            let id = names(&module)
                .into_iter()
                .find(|(_, n)| n == name)
                .map(|(id, _)| id)
                .unwrap();
            decoration_value(&module, id, Decoration::Binding).unwrap()
        };
        let block = module
            .types_global_values
            .iter()
            .find(|inst| {
                inst.class.opcode == Op::Variable
                    && matches!(
                        inst.operands.first(),
                        Some(Operand::StorageClass(StorageClass::Uniform))
                    )
            })
            .and_then(|inst| inst.result_id)
            .unwrap();
        assert_eq!(
            decoration_value(&module, block, Decoration::Binding),
            Some(0)
        );
        // SPIR-V order is glslang's emission order (live resources first).
        let mut samplers = [binding_of("u_tex"), binding_of("u_orig")];
        samplers.sort();
        assert_eq!(
            samplers,
            [1, 2],
            "the unused sampler no longer shares binding 0 with the block"
        );
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

    #[test]
    fn non_finite_constants_reach_naga_as_runtime_values() {
        let src = "#version 450\nlayout(location=0) in vec2 v_st;\nout vec4 o;\nconst float INF = 1.0 / 0.0;\nconst vec3 FAR = vec3(1e39, 0.0, -1e39);\nfloat far(float x){ return min(x, INF); }\nvoid main(){ vec3 f = FAR; float n = 0.0 / 0.0; o = vec4(far(v_st.x), f.x > 1.0 ? 1.0 : 0.0, isinf(f.z) ? 1.0 : 0.0, isnan(n) ? 1.0 : 0.5); }\n";
        let spv = compile(src, Stage::Fragment).unwrap();
        let mut module = load(&spv.words).unwrap();
        assert!(materialize_non_finite_constants(&mut module) >= 2);
        let words = assemble(&module);
        let options = naga::front::spv::Options {
            adjust_coordinate_space: true,
            strict_capabilities: false,
            block_ctx_dump_prefix: None,
        };
        let naga_module = naga::front::spv::Frontend::new(words.into_iter(), &options)
            .parse()
            .expect("naga parses");
        let info = crate::pipeline::naga_validate(&naga_module).expect("naga validates");
        let (msl, _) = naga::back::msl::write_string(
            &naga_module,
            &info,
            &Default::default(),
            &Default::default(),
        )
        .expect("msl");
        assert!(msl.contains("as_type<float>"), "{msl}");
    }
}
