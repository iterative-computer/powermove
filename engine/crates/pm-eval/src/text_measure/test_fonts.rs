//! Tiny synthetic TrueType fonts and a resolver over them, so cache tests
//! run on any host without platform fonts.

use pm_text::font::{FaceFacts, FaceId, FaceInfo, FontBytes, FontDesc, FontResolver};

/// A font whose printable ASCII glyphs are `width`-unit boxes on a
/// `advance`-unit advance; space (glyph 1) and U+2003 ([`EM_SPACE`]) map to
/// empty glyphs with an advance (no outline), `usWeightClass` is `weight`.
pub(crate) fn font(weight: u16, advance: u16, width: i16) -> Vec<u8> {
    let mut tables = tables(weight, advance, width);
    pm_text::sfnt::build_sfnt(&mut tables)
}

/// A collection file holding one font per entry of `fonts`.
pub(crate) fn collection(fonts: &[(u16, u16, i16)]) -> Vec<u8> {
    let faces: Vec<Vec<(u32, Vec<u8>)>> = fonts
        .iter()
        .map(|&(w, a, x)| {
            let mut t = tables(w, a, x);
            t.sort_by_key(|(tag, _)| *tag);
            t
        })
        .collect();
    let header = 12 + 4 * faces.len();
    let directories: Vec<usize> = faces.iter().map(|t| 12 + 16 * t.len()).collect();
    let mut offset = header + directories.iter().sum::<usize>();
    let mut out = Vec::new();
    out.extend_from_slice(b"ttcf");
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&(faces.len() as u32).to_be_bytes());
    let mut at = header;
    for d in &directories {
        out.extend_from_slice(&(at as u32).to_be_bytes());
        at += d;
    }
    let mut data = Vec::new();
    for tables in &faces {
        let n = tables.len() as u16;
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        for (tag, bytes) in tables {
            out.extend_from_slice(&tag.to_be_bytes());
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            data.extend_from_slice(bytes);
            while data.len() % 4 != 0 {
                data.push(0);
            }
            offset = header + directories.iter().sum::<usize>() + data.len();
        }
    }
    out.extend_from_slice(&data);
    out
}

const GLYPHS: u16 = 3 + (0x7e - 0x21 + 1);
pub(crate) const EM_SPACE: u32 = GLYPHS as u32 - 1;

fn tables(weight: u16, advance: u16, width: i16) -> Vec<(u32, Vec<u8>)> {
    let tag = |t: &[u8; 4]| u32::from_be_bytes(*t);
    let mut head = Vec::new();
    head.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    head.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    head.extend_from_slice(&0u32.to_be_bytes());
    head.extend_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head.extend_from_slice(&0u16.to_be_bytes());
    head.extend_from_slice(&1000u16.to_be_bytes());
    head.extend_from_slice(&[0; 16]);
    for v in [0i16, 0, width + 50, 700] {
        head.extend_from_slice(&v.to_be_bytes());
    }
    head.extend_from_slice(&0u16.to_be_bytes());
    head.extend_from_slice(&8u16.to_be_bytes());
    head.extend_from_slice(&2i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());
    head.extend_from_slice(&0i16.to_be_bytes());

    let mut hhea = Vec::new();
    hhea.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    for v in [800i16, -200, 0] {
        hhea.extend_from_slice(&v.to_be_bytes());
    }
    hhea.extend_from_slice(&advance.to_be_bytes());
    for v in [0i16, 0, width + 50, 1, 0, 0, 0, 0, 0, 0, 0] {
        hhea.extend_from_slice(&v.to_be_bytes());
    }
    hhea.extend_from_slice(&GLYPHS.to_be_bytes());

    let mut maxp = Vec::new();
    maxp.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp.extend_from_slice(&GLYPHS.to_be_bytes());
    for v in [4u16, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        maxp.extend_from_slice(&v.to_be_bytes());
    }

    let mut os2 = Vec::new();
    os2.extend_from_slice(&0u16.to_be_bytes());
    os2.extend_from_slice(&(advance as i16).to_be_bytes());
    os2.extend_from_slice(&weight.to_be_bytes());
    os2.extend_from_slice(&5u16.to_be_bytes());
    os2.extend_from_slice(&[0; 2 + 20 + 2 + 10 + 16]);
    os2.extend_from_slice(b"TEST");
    let selection: u16 = if weight >= 700 { 0x20 } else { 0x40 };
    os2.extend_from_slice(&selection.to_be_bytes());
    os2.extend_from_slice(&0x20u16.to_be_bytes());
    os2.extend_from_slice(&0x2003u16.to_be_bytes());
    for v in [800i16, -200, 0] {
        os2.extend_from_slice(&v.to_be_bytes());
    }
    os2.extend_from_slice(&800u16.to_be_bytes());
    os2.extend_from_slice(&200u16.to_be_bytes());

    let mut hmtx = Vec::new();
    for _ in 0..GLYPHS {
        hmtx.extend_from_slice(&advance.to_be_bytes());
        hmtx.extend_from_slice(&50i16.to_be_bytes());
    }

    // Format 12: space to glyph 1, printable ASCII to 2..., U+2003 last.
    let groups: [(u32, u32, u32); 3] =
        [(0x20, 0x20, 1), (0x21, 0x7e, 2), (0x2003, 0x2003, EM_SPACE)];
    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&1u16.to_be_bytes());
    cmap.extend_from_slice(&3u16.to_be_bytes());
    cmap.extend_from_slice(&10u16.to_be_bytes());
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend_from_slice(&12u16.to_be_bytes());
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&(16 + 12 * groups.len() as u32).to_be_bytes());
    cmap.extend_from_slice(&0u32.to_be_bytes());
    cmap.extend_from_slice(&(groups.len() as u32).to_be_bytes());
    for (a, b, g) in groups {
        for v in [a, b, g] {
            cmap.extend_from_slice(&v.to_be_bytes());
        }
    }

    let boxed = |x1: i16| {
        let mut g = Vec::new();
        for v in [1i16, 50, 0, x1, 700] {
            g.extend_from_slice(&v.to_be_bytes());
        }
        g.extend_from_slice(&3u16.to_be_bytes());
        g.extend_from_slice(&0u16.to_be_bytes());
        g.extend_from_slice(&[1; 4]);
        for v in [50i16, 0, x1 - 50, 0, 0, 700, 0, -700] {
            g.extend_from_slice(&v.to_be_bytes());
        }
        g
    };
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for gid in 0..GLYPHS {
        loca.extend_from_slice(&((glyf.len() / 2) as u16).to_be_bytes());
        if gid != 1 && u32::from(gid) != EM_SPACE {
            glyf.extend_from_slice(&boxed(50 + width));
        }
    }
    loca.extend_from_slice(&((glyf.len() / 2) as u16).to_be_bytes());

    let name = [0u8, 0, 0, 0, 0, 6].to_vec();
    let mut post = Vec::new();
    post.extend_from_slice(&0x0003_0000u32.to_be_bytes());
    post.extend_from_slice(&[0; 28]);

    vec![
        (tag(b"head"), head),
        (tag(b"hhea"), hhea),
        (tag(b"maxp"), maxp),
        (tag(b"OS/2"), os2),
        (tag(b"hmtx"), hmtx),
        (tag(b"cmap"), cmap),
        (tag(b"glyf"), glyf),
        (tag(b"loca"), loca),
        (tag(b"name"), name),
        (tag(b"post"), post),
    ]
}

/// A resolver over named families of synthetic faces. A family resolves the
/// face with the nearest weight at or below the request, else its first
/// face; unknown families resolve face 0.
pub(crate) struct Fonts {
    pub faces: Vec<FaceInfo>,
    pub families: Vec<(String, u16, FaceId)>,
    /// Requests at or above this weight take the family's heaviest face,
    /// whatever CSS matching would choose.
    pub bold_from: f32,
}

impl Fonts {
    pub fn new() -> Self {
        Fonts {
            faces: Vec::new(),
            families: Vec::new(),
            bold_from: 600.0,
        }
    }

    pub fn add(
        &mut self,
        family: &str,
        weight: u16,
        blob: &str,
        data: FontBytes,
        index: u32,
    ) -> FaceId {
        let id = self.faces.len() as FaceId;
        self.faces.push(FaceInfo {
            data,
            index,
            blob: blob.into(),
            coords: vec![],
            synthetic_bold: false,
            synthetic_italic: false,
            custom: false,
            web: None,
            synthesized_hyphens: [None, None],
            postscript_name: blob.into(),
            typeface_key: format!("{blob}#{index}"),
            fallback_key: None,
            facts: FaceFacts::default(),
        });
        self.families.push((family.into(), weight, id));
        id
    }
}

impl FontResolver for Fonts {
    fn face(&self, id: FaceId) -> &FaceInfo {
        &self.faces[id as usize]
    }
    fn family(&mut self, family: &str, desc: &FontDesc) -> Option<FaceId> {
        let mut faces: Vec<_> = self.families.iter().filter(|f| f.0 == family).collect();
        faces.sort_by_key(|f| f.1);
        if desc.weight >= self.bold_from {
            return faces.last().map(|f| f.2);
        }
        faces
            .iter()
            .rev()
            .find(|f| f32::from(f.1) <= desc.weight)
            .or(faces.first())
            .map(|f| f.2)
    }
    fn standard(&mut self, _: &FontDesc) -> Option<FaceId> {
        (!self.faces.is_empty()).then_some(0)
    }
    fn fallback_for_char(&mut self, _: &FontDesc, _: u32, _: FaceId, _: bool) -> Option<FaceId> {
        None
    }
    fn last_resort(&mut self, _: &FontDesc) -> Option<FaceId> {
        None
    }
    fn variable_face(&mut self, _: &str) -> Option<FaceId> {
        None
    }
}
