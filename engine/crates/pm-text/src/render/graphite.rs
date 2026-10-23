//! Device-space Graphite draws retained until the native GPU paint pass.
//! The image paired with this packet is the CPU renderer's reference;
//! native Metal uses these primitives and the source render-step shaders.

#[derive(Clone, Debug)]
pub enum Geometry {
    /// AnalyticRRectRenderStep instance attributes, followed by mat0..2.
    Analytic([[f32; 4]; 7]),
    /// Triangle union, with one blend per covered sample.
    Triangles(Vec<[f32; 2]>),
    /// Contour fans, stencilled under nonzero or even-odd winding.
    Winding {
        vertices: Vec<[f32; 2]>,
        even_odd: bool,
    },
    /// Local-space patches evaluated by tessellate_filled_curve on Metal.
    Fill {
        vertex_count: u32,
        patches: Vec<[[f32; 4]; 5]>,
        winding: bool,
        even_odd: bool,
    },
    /// The A8-derived distance field and the source text-vertex transform.
    Sdf {
        /// Blink keeps a platform typeface/strike per CSS font size.
        font_size: u32,
        field: std::sync::Arc<super::sdf::SdfGlyph>,
        left_top: (f32, f32),
        matrix: [f32; 6],
    },
    /// Local-space patches evaluated by tessellate_stroked_curve on Metal.
    Stroke {
        patches: Vec<[[f32; 4]; 5]>,
        vertex_count: u32,
    },
}

#[derive(Clone, Debug)]
pub struct Draw {
    pub geometry: Geometry,
    pub color: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub crop: [u32; 4],
    pub multisampled: bool,
    pub draws: Vec<Draw>,
}

impl Raster {
    pub fn hash_content(&self, h: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        (self.width, self.height, self.crop, self.multisampled).hash(h);
        self.draws.len().hash(h);
        for draw in &self.draws {
            draw.color.map(f32::to_bits).hash(h);
            match &draw.geometry {
                Geometry::Analytic(a) => {
                    0u8.hash(h);
                    a.map(|v| v.map(f32::to_bits)).hash(h);
                }
                Geometry::Triangles(vertices) => {
                    1u8.hash(h);
                    vertices.len().hash(h);
                    for v in vertices {
                        v.map(f32::to_bits).hash(h);
                    }
                }
                Geometry::Winding { vertices, even_odd } => {
                    2u8.hash(h);
                    even_odd.hash(h);
                    vertices.len().hash(h);
                    for v in vertices {
                        v.map(f32::to_bits).hash(h);
                    }
                }
                Geometry::Fill {
                    patches,
                    vertex_count,
                    winding,
                    even_odd,
                } => {
                    (3u8, vertex_count, winding, even_odd).hash(h);
                    patches.len().hash(h);
                    for p in patches {
                        p.map(|v| v.map(f32::to_bits)).hash(h);
                    }
                }
                Geometry::Sdf {
                    font_size,
                    field,
                    left_top,
                    matrix,
                } => {
                    5u8.hash(h);
                    font_size.hash(h);
                    (field.left, field.top, field.width, field.height).hash(h);
                    field.field.hash(h);
                    (left_top.0.to_bits(), left_top.1.to_bits()).hash(h);
                    matrix.map(f32::to_bits).hash(h);
                }
                Geometry::Stroke {
                    patches,
                    vertex_count,
                } => {
                    (4u8, vertex_count).hash(h);
                    patches.len().hash(h);
                    for p in patches {
                        p.map(|v| v.map(f32::to_bits)).hash(h);
                    }
                }
            }
        }
    }
}
