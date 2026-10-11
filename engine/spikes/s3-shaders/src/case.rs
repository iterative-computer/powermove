//! The corpus format written by `scripts/collect.ts`. Every pass lists the
//! exact uniform calls the legacy compositor issues (`setU`/`setI`) and which
//! image sits on each texture unit, so the WebGL runner and this runner replay
//! the same sequence.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Corpus {
    pub vertex: String,
    pub images: Vec<ImageSpec>,
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ImageSpec {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Raw little-endian f16 RGBA, rows bottom-up (GL row order).
    pub file: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Case {
    pub id: String,
    pub group: String,
    pub kind: String,
    pub label: String,
    pub source: String,
    pub frag: String,
    /// Program lines before the author's first line (for diagnostics).
    #[serde(rename = "lineOffset")]
    pub line_offset: u32,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Variant {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub passes: Vec<Pass>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Pass {
    pub calls: Vec<Call>,
    pub units: Vec<Unit>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Call {
    /// `1f`, `2f`, `3f`, `4f`, `1i`, or `m3`.
    #[serde(rename = "fn")]
    pub func: String,
    pub name: String,
    /// Positional fallback (`u_p<i>`/`u_c<i>`): used when `name` is not an
    /// active uniform, as the compositor's `setParam` decides with
    /// `getUniformLocation`.
    #[serde(default)]
    pub alt: Option<String>,
    pub v: Vec<f64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Unit {
    pub unit: u32,
    /// An image name, or `pass:<k>` for the output of pass k.
    pub src: String,
}
