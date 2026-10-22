//! Constant tables indexed at run time read from the program's constants.
//!
//! GLSL ES lets a program index a `const` array with a run-time index (a
//! glyph bitmap table, a palette). glslang lowers each such read to a
//! function-local copy of the whole array, stored before it is indexed, and
//! naga's MSL keeps that copy: every fragment wrote the whole table into its
//! own memory before reading one entry. A user effect with an 1,104-entry
//! glyph table took 1.5 s of GPU per 1080p frame that way (WebGL's ANGLE
//! reads it from Metal's constant address space and draws it in
//! milliseconds).
//!
//! This pass finds such locals: an array variable whose every store writes
//! the same module constant, stored before anything reads it, and otherwise
//! only read through element accesses. Its reads then index the constant
//! itself (naga writes it once, in program scope), and its stores go. The
//! values every read sees are the same, so the pixels are too. A module the
//! validator refuses afterwards is left as it was (the caller checks).

use naga::{Block, Expression, Function, Handle, LocalVariable, Module, Statement, TypeInner};

/// Applies the pass to every function; returns how many tables it moved.
pub fn hoist(module: &mut Module) -> usize {
    let types = &module.types;
    let constants = &module.constants;
    let mut tables = Vec::new();
    for (_, function) in module.functions.iter_mut() {
        tables.extend(hoist_in(function, types, constants));
    }
    for entry in module.entry_points.iter_mut() {
        tables.extend(hoist_in(&mut entry.function, types, constants));
    }
    // A named constant is written once, in program scope (`constant`
    // address space); an unnamed one is written inline at each use.
    for (i, c) in tables.iter().enumerate() {
        if module.constants[*c].name.is_none() {
            module.constants.get_mut(*c).name = Some(format!("pm_table_{i}"));
        }
    }
    tables.len()
}

/// What a variable's uses look like.
#[derive(Default)]
struct Uses {
    /// The constant every whole store writes (`None`: none seen yet).
    constant: Option<Handle<naga::Constant>>,
    /// Disqualified: a partial write, another value, a read before the
    /// first store, or a use other than an element read.
    bad: bool,
    stored: bool,
}

fn hoist_in(
    function: &mut Function,
    types: &naga::UniqueArena<naga::Type>,
    constants: &naga::Arena<naga::Constant>,
) -> Vec<Handle<naga::Constant>> {
    // Array locals without an initializer.
    let candidates: Vec<Handle<LocalVariable>> = function
        .local_variables
        .iter()
        .filter(|(_, v)| v.init.is_none() && matches!(types[v.ty].inner, TypeInner::Array { .. }))
        .map(|(h, _)| h)
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let exprs = &function.expressions;
    // The variable an expression's pointer chain starts at, and whether the
    // chain is only element accesses.
    let root = |mut h: Handle<Expression>| -> Option<(Handle<LocalVariable>, bool)> {
        let mut direct = true;
        loop {
            match exprs[h] {
                Expression::LocalVariable(v) => return Some((v, direct)),
                Expression::Access { base, .. } | Expression::AccessIndex { base, .. } => {
                    direct = false;
                    h = base;
                }
                _ => return None,
            }
        }
    };
    let mut uses: std::collections::HashMap<Handle<LocalVariable>, Uses> =
        candidates.iter().map(|&v| (v, Uses::default())).collect();
    // In naga only element accesses, loads (and `arrayLength`, for runtime
    // arrays) take a pointer operand among expressions; statements that do
    // (calls, atomics) are refused in `walk`.
    // Program order: the first use must be a whole store of a constant.
    fn walk(
        block: &Block,
        exprs: &naga::Arena<Expression>,
        root: &dyn Fn(Handle<Expression>) -> Option<(Handle<LocalVariable>, bool)>,
        uses: &mut std::collections::HashMap<Handle<LocalVariable>, Uses>,
    ) {
        for statement in block.iter() {
            match statement {
                Statement::Emit(range) => {
                    for h in range.clone() {
                        if let Expression::Load { pointer } = exprs[h]
                            && let Some((v, _)) = root(pointer)
                            && let Some(u) = uses.get_mut(&v)
                            && !u.stored
                        {
                            u.bad = true;
                        }
                    }
                }
                Statement::Store { pointer, value } => {
                    if let Some((v, direct)) = root(*pointer)
                        && let Some(u) = uses.get_mut(&v)
                    {
                        match (direct, &exprs[*value]) {
                            (true, Expression::Constant(c)) if u.constant.is_none_or(|k| k == *c) => {
                                u.constant = Some(*c);
                                u.stored = true;
                            }
                            _ => u.bad = true,
                        }
                    }
                }
                Statement::Block(b) => walk(b, exprs, root, uses),
                Statement::If { accept, reject, .. } => {
                    walk(accept, exprs, root, uses);
                    walk(reject, exprs, root, uses);
                }
                Statement::Switch { cases, .. } => {
                    for case in cases {
                        walk(&case.body, exprs, root, uses);
                    }
                }
                Statement::Loop {
                    body, continuing, ..
                } => {
                    walk(body, exprs, root, uses);
                    walk(continuing, exprs, root, uses);
                }
                Statement::Call { arguments, .. } => {
                    for argument in arguments {
                        if let Some((v, _)) = root(*argument)
                            && let Some(u) = uses.get_mut(&v)
                        {
                            u.bad = true;
                        }
                    }
                }
                Statement::Atomic { pointer, .. } => {
                    if let Some((v, _)) = root(*pointer)
                        && let Some(u) = uses.get_mut(&v)
                    {
                        u.bad = true;
                    }
                }
                _ => {}
            }
        }
    }
    walk(&function.body, exprs, &root, &mut uses);
    let hoisted: std::collections::HashMap<Handle<LocalVariable>, Handle<naga::Constant>> = uses
        .into_iter()
        .filter_map(|(v, u)| match (u.bad, u.constant) {
            (false, Some(c)) if constants[c].ty == function.local_variables[v].ty => Some((v, c)),
            _ => None,
        })
        .collect();
    if hoisted.is_empty() {
        return Vec::new();
    }
    // Reads index the constant: the variable becomes the constant's value,
    // each load of an element becomes the element.
    let handles: Vec<Handle<Expression>> = function.expressions.iter().map(|(h, _)| h).collect();
    for &h in &handles {
        if let Expression::LocalVariable(v) = function.expressions[h]
            && let Some(&c) = hoisted.get(&v)
        {
            function.expressions[h] = Expression::Constant(c);
        }
    }
    for &h in &handles {
        if let Expression::Load { pointer } = function.expressions[h] {
            let mut base = pointer;
            let rooted = loop {
                match function.expressions[base] {
                    Expression::Constant(c) => break hoisted.values().any(|&k| k == c),
                    Expression::Access { base: b, .. } | Expression::AccessIndex { base: b, .. } => base = b,
                    _ => break false,
                }
            };
            if rooted {
                function.expressions[h] = function.expressions[pointer].clone();
            }
        }
    }
    // The stores go.
    let constant_values: Vec<Handle<naga::Constant>> = hoisted.values().copied().collect();
    remove_stores(&mut function.body, &function.expressions, &constant_values);
    constant_values
}

/// Removes whole stores whose pointer is now one of the hoisted constants.
fn remove_stores(block: &mut Block, exprs: &naga::Arena<Expression>, hoisted: &[Handle<naga::Constant>]) {
    let old = std::mem::take(block);
    for (mut statement, span) in old.span_into_iter() {
        match &mut statement {
            Statement::Store { pointer, .. } => {
                if let Expression::Constant(c) = exprs[*pointer]
                    && hoisted.contains(&c)
                {
                    continue;
                }
            }
            Statement::Block(b) => remove_stores(b, exprs, hoisted),
            Statement::If { accept, reject, .. } => {
                remove_stores(accept, exprs, hoisted);
                remove_stores(reject, exprs, hoisted);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    remove_stores(&mut case.body, exprs, hoisted);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                remove_stores(body, exprs, hoisted);
                remove_stores(continuing, exprs, hoisted);
            }
            _ => {}
        }
        block.push(statement, span);
    }
}
