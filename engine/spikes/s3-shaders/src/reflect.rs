//! Reflection over glslang's SPIR-V: the default uniform block layout, which
//! members are statically used (GL "active" uniforms), and sampler bindings.
//! Used to port the legacy compositor's uniform binding exactly.

use std::collections::{HashMap, HashSet};

use rspirv::dr::{Module, Operand};
use rspirv::spirv::{Decoration, Op, StorageClass, Word};

use crate::rewrite::names;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    Float,
    Int,
    Uint,
    /// A `uint` block member that the source declared `bool` (relaxed rules lowering).
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UniformType {
    Scalar(Scalar),
    Vector(Scalar, u32),
    /// Column-major `columns` x `rows` float matrix.
    Matrix { columns: u32, rows: u32, stride: u32 },
    Array { element: Box<UniformType>, length: u32, stride: u32 },
    Struct,
}

#[derive(Debug, Clone)]
pub struct Member {
    pub name: String,
    pub offset: u32,
    pub ty: UniformType,
    /// Statically referenced by an access chain (GL's "active" approximation).
    pub active: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub set: u32,
    pub binding: u32,
    pub size: u32,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone)]
pub struct Sampler {
    pub name: String,
    pub set: u32,
    pub binding: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Reflection {
    pub block: Option<Block>,
    pub samplers: Vec<Sampler>,
}

fn decorations(module: &Module, target: Word) -> HashMap<Decoration, Option<u32>> {
    let mut out = HashMap::new();
    for inst in &module.annotations {
        if inst.class.opcode == Op::Decorate && inst.operands.first() == Some(&Operand::IdRef(target)) {
            if let Some(Operand::Decoration(d)) = inst.operands.get(1) {
                let value = match inst.operands.get(2) {
                    Some(Operand::LiteralBit32(v)) => Some(*v),
                    _ => None,
                };
                out.insert(*d, value);
            }
        }
    }
    out
}

fn member_decoration(module: &Module, target: Word, member: u32, decoration: Decoration) -> Option<u32> {
    module.annotations.iter().find_map(|inst| {
        if inst.class.opcode != Op::MemberDecorate || inst.operands.first() != Some(&Operand::IdRef(target)) {
            return None;
        }
        match (inst.operands.get(1), inst.operands.get(2), inst.operands.get(3)) {
            (Some(Operand::LiteralBit32(m)), Some(Operand::Decoration(d)), value) if *m == member && *d == decoration => match value {
                Some(Operand::LiteralBit32(v)) => Some(*v),
                _ => Some(0),
            },
            _ => None,
        }
    })
}

fn member_names(module: &Module, target: Word) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for inst in &module.debug_names {
        if inst.class.opcode == Op::MemberName && inst.operands.first() == Some(&Operand::IdRef(target)) {
            if let (Some(Operand::LiteralBit32(m)), Some(Operand::LiteralString(name))) = (inst.operands.get(1), inst.operands.get(2)) {
                out.insert(*m, name.clone());
            }
        }
    }
    out
}

struct Types<'a> {
    module: &'a Module,
    by_id: HashMap<Word, &'a rspirv::dr::Instruction>,
    constants: HashMap<Word, u32>,
}

impl<'a> Types<'a> {
    fn new(module: &'a Module) -> Self {
        let mut by_id = HashMap::new();
        let mut constants = HashMap::new();
        for inst in &module.types_global_values {
            if let Some(id) = inst.result_id {
                by_id.insert(id, inst);
                if inst.class.opcode == Op::Constant {
                    if let Some(Operand::LiteralBit32(v)) = inst.operands.first() {
                        constants.insert(id, *v);
                    }
                }
            }
        }
        Self { module, by_id, constants }
    }

    fn scalar(&self, id: Word) -> Option<Scalar> {
        let inst = self.by_id.get(&id)?;
        match inst.class.opcode {
            Op::TypeFloat => Some(Scalar::Float),
            Op::TypeInt => match inst.operands.get(1) {
                Some(Operand::LiteralBit32(1)) => Some(Scalar::Int),
                _ => Some(Scalar::Uint),
            },
            Op::TypeBool => Some(Scalar::Bool),
            _ => None,
        }
    }

    fn ty(&self, id: Word, matrix_stride: Option<u32>) -> UniformType {
        let Some(inst) = self.by_id.get(&id) else { return UniformType::Struct };
        let id_at = |k: usize| match inst.operands.get(k) {
            Some(Operand::IdRef(i)) => *i,
            _ => 0,
        };
        let lit_at = |k: usize| match inst.operands.get(k) {
            Some(Operand::LiteralBit32(v)) => *v,
            _ => 0,
        };
        match inst.class.opcode {
            Op::TypeFloat | Op::TypeInt | Op::TypeBool => UniformType::Scalar(self.scalar(id).unwrap()),
            Op::TypeVector => UniformType::Vector(self.scalar(id_at(0)).unwrap_or(Scalar::Float), lit_at(1)),
            Op::TypeMatrix => {
                let column = id_at(0);
                let rows = match self.by_id.get(&column).and_then(|c| c.operands.get(1)) {
                    Some(Operand::LiteralBit32(r)) => *r,
                    _ => 4,
                };
                UniformType::Matrix { columns: lit_at(1), rows, stride: matrix_stride.unwrap_or(16) }
            }
            Op::TypeArray => {
                let length = self.constants.get(&id_at(1)).copied().unwrap_or(0);
                let stride = decorations(self.module, id).get(&Decoration::ArrayStride).copied().flatten().unwrap_or(16);
                UniformType::Array { element: Box::new(self.ty(id_at(0), matrix_stride)), length, stride }
            }
            _ => UniformType::Struct,
        }
    }
}

pub fn size_of(ty: &UniformType) -> u32 {
    match ty {
        UniformType::Scalar(_) => 4,
        UniformType::Vector(_, n) => 4 * n,
        UniformType::Matrix { columns, stride, .. } => columns * stride,
        UniformType::Array { length, stride, .. } => length * stride,
        UniformType::Struct => 16,
    }
}

/// `bool_names`: uniforms the (preprocessed) source declares as `bool`.
pub fn reflect(module: &Module, bool_names: &HashSet<String>) -> Reflection {
    let types = Types::new(module);
    let names = names(module);
    let mut reflection = Reflection::default();

    // Block members statically used by access chains inside functions.
    let mut used_members: HashMap<Word, HashSet<u32>> = HashMap::new();
    for function in &module.functions {
        for block in &function.blocks {
            for inst in &block.instructions {
                if matches!(inst.class.opcode, Op::AccessChain | Op::InBoundsAccessChain) {
                    if let (Some(Operand::IdRef(base)), Some(Operand::IdRef(index))) = (inst.operands.first(), inst.operands.get(1)) {
                        if let Some(&member) = types.constants.get(index) {
                            used_members.entry(*base).or_default().insert(member);
                        }
                    }
                }
            }
        }
    }

    for inst in &module.types_global_values {
        if inst.class.opcode != Op::Variable {
            continue;
        }
        let id = inst.result_id.unwrap();
        let pointer = types.by_id.get(&inst.result_type.unwrap());
        let pointee = pointer.and_then(|p| match p.operands.get(1) {
            Some(Operand::IdRef(t)) => Some(*t),
            _ => None,
        });
        let storage = inst.operands.first();
        let deco = decorations(module, id);
        let set = deco.get(&Decoration::DescriptorSet).copied().flatten().unwrap_or(0);
        let binding = deco.get(&Decoration::Binding).copied().flatten().unwrap_or(0);
        match storage {
            Some(Operand::StorageClass(StorageClass::Uniform)) => {
                let Some(struct_id) = pointee else { continue };
                let Some(struct_inst) = types.by_id.get(&struct_id) else { continue };
                if struct_inst.class.opcode != Op::TypeStruct {
                    continue;
                }
                let member_names = member_names(module, struct_id);
                let used = used_members.get(&id).cloned().unwrap_or_default();
                let mut members = Vec::new();
                let mut size = 0;
                for (index, op) in struct_inst.operands.iter().enumerate() {
                    let Operand::IdRef(member_type) = op else { continue };
                    let index = index as u32;
                    let offset = member_decoration(module, struct_id, index, Decoration::Offset).unwrap_or(0);
                    let stride = member_decoration(module, struct_id, index, Decoration::MatrixStride);
                    let name = member_names.get(&index).cloned().unwrap_or_default();
                    let mut ty = types.ty(*member_type, stride);
                    if ty == UniformType::Scalar(Scalar::Uint) && bool_names.contains(&name) {
                        ty = UniformType::Scalar(Scalar::Bool);
                    }
                    size = size.max(offset + size_of(&ty));
                    members.push(Member { name, offset, ty, active: used.contains(&index) });
                }
                reflection.block = Some(Block { set, binding, size: size.div_ceil(16) * 16, members });
            }
            Some(Operand::StorageClass(StorageClass::UniformConstant)) => {
                let is_sampled = pointee.and_then(|t| types.by_id.get(&t)).is_some_and(|t| t.class.opcode == Op::TypeSampledImage);
                if is_sampled {
                    reflection.samplers.push(Sampler {
                        name: names.get(&id).cloned().unwrap_or_default(),
                        set,
                        binding,
                    });
                }
            }
            _ => {}
        }
    }
    reflection
}

/// Names declared `uniform bool` in preprocessed GLSL (`uniform bool a, b;`).
pub fn bool_uniforms(preprocessed: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let tokens: Vec<&str> = preprocessed.split(|c: char| c.is_whitespace()).filter(|t| !t.is_empty()).collect();
    let text = tokens.join(" ");
    for statement in text.split(';') {
        let statement = statement.trim();
        let Some(rest) = statement.strip_prefix("uniform ") else {
            // `layout(...) uniform bool x` and precision qualifiers.
            if let Some(index) = statement.find(" uniform ") {
                collect_bools(&statement[index + 9..], &mut out);
            }
            continue;
        };
        collect_bools(rest, &mut out);
    }
    out
}

fn collect_bools(rest: &str, out: &mut HashSet<String>) {
    let rest = rest.trim_start_matches("lowp ").trim_start_matches("mediump ").trim_start_matches("highp ");
    if let Some(names) = rest.strip_prefix("bool ") {
        for name in names.split(',') {
            let name = name.split(['[', '=']).next().unwrap_or("").trim();
            if !name.is_empty() {
                out.insert(name.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{Stage, compile};

    #[test]
    fn reflects_block_layout_bools_and_activity() {
        let src = "#version 450\nuniform bool u_flag, u_other; uniform float u_unused; uniform mat3 u_m; uniform vec3 u_c; uniform sampler2D u_tex;\nout vec4 o; void main(){ o = texture(u_tex, vec2(0.5)) * float(u_flag) + vec4(u_m * u_c, 1.0); }\n";
        let spv = compile(src, Stage::Fragment).unwrap();
        let module = crate::rewrite::load(&spv.words).unwrap();
        let reflection = reflect(&module, &bool_uniforms(&spv.preprocessed));
        let block = reflection.block.unwrap();
        let get = |n: &str| block.members.iter().find(|m| m.name == n).unwrap().clone();
        assert_eq!(get("u_flag").ty, UniformType::Scalar(Scalar::Bool));
        assert_eq!(get("u_other").ty, UniformType::Scalar(Scalar::Bool));
        assert!(get("u_flag").active);
        assert!(!get("u_unused").active);
        assert_eq!(get("u_m").ty, UniformType::Matrix { columns: 3, rows: 3, stride: 16 });
        assert_eq!(reflection.samplers.len(), 1);
        assert_eq!(reflection.samplers[0].name, "u_tex");
    }
}
