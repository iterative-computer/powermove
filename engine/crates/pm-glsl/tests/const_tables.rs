//! Constant tables indexed at run time (`pm_glsl::const_tables`): read from a
//! program-scope `constant` array in the MSL, never copied into each
//! fragment's memory, as WebGL's ANGLE reads them.

fn table(n: usize) -> String {
    (0..n)
        .map(|i| format!("{}u", (i as u64 * 2_654_435_761) % 4_294_967_291))
        .collect::<Vec<_>>()
        .join(",")
}

fn msl(body: &str) -> String {
    let frag = format!(
        "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nout vec4 o;\n{body}"
    );
    let program = pm_glsl::pipeline::translate(&frag, 6).unwrap_or_else(|f| panic!("{}: {}", f.stage.name(), f.message));
    pm_glsl::pipeline::msl(&program.fragment.module).expect("msl")
}

#[test]
fn a_constant_table_indexed_at_run_time_is_read_from_program_constants() {
    let out = msl(&format!(
        "const uint glyphBits[1104] = uint[]({});\nvoid main() {{\n  int i = int(v_uv.x * 1103.0);\n  uint bits = glyphBits[i];\n  o = vec4(float(bits & 255u) / 255.0, v_uv, 1.0);\n}}\n",
        table(1104)
    ));
    let declaration = out.lines().find(|l| l.starts_with("constant ") && l.contains("pm_table_")).expect("a program-scope constant table");
    assert!(declaration.contains("2654435761u"));
    // No fragment writes the table into its own memory.
    assert_eq!(out.matches("2654435761u").count(), 1, "{out}");
}

#[test]
fn tables_read_in_loops_and_functions_are_moved_too() {
    let out = msl(&format!(
        "const uint lut[256] = uint[]({});\nuint pick(int i) {{ return lut[i & 255]; }}\nvoid main() {{\n  uint acc = 0u;\n  for (int k = 0; k < 8; k++) acc ^= lut[(int(v_uv.x * 255.0) + k) & 255] ^ pick(k * 31);\n  o = vec4(float(acc & 255u) / 255.0, v_uv, 1.0);\n}}\n",
        table(256)
    ));
    assert!(out.lines().any(|l| l.starts_with("constant ") && l.contains("pm_table_")), "{out}");
    assert_eq!(out.matches("2654435761u").count(), 1, "one copy, in program scope:\n{out}");
}

#[test]
fn a_local_array_the_program_writes_stays_local() {
    // Written after its initial value: a real variable, not a table.
    let out = msl(
        "void main() {\n  float a[4] = float[](1.0, 2.0, 3.0, 4.0);\n  a[int(v_uv.y * 3.0)] = v_uv.x;\n  o = vec4(a[int(v_uv.x * 3.0)], v_uv, 1.0);\n}\n",
    );
    assert!(!out.contains("pm_table_"), "{out}");
}
