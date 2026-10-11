//! VideoToolbox output formats per codec: which requested formats succeed, the
//! delivered format, hardware use, attachments, and the sample layout of 12-bit
//! ProRes in y416.

use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use crate::cf::{self, Result};
use crate::material::Material;
use crate::pixbuf;
use crate::vt;

#[derive(Serialize)]
pub struct FormatRow {
    pub source: String,
    pub codec: String,
    pub requested: String,
    pub ok: bool,
    pub delivered: Option<String>,
    pub hardware: Option<bool>,
    pub ms_per_frame: Option<f64>,
    pub attachments: Vec<(String, String)>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct Y416Layout {
    pub source: String,
    /// Fraction of Y′/Cb/Cr samples whose low 4 bits are zero (pure 12-bit << 4).
    pub low4_zero_fraction: f64,
    /// Fraction of samples equal to (v >> 4) * 16 + (v >> 12), i.e. bit replication.
    pub bit_replicated_fraction: f64,
    pub y_min: u16,
    pub y_max: u16,
    pub c_min: u16,
    pub c_max: u16,
    pub alpha_distinct: usize,
    pub alpha_low4_zero_fraction: f64,
}

#[derive(Serialize)]
pub struct FormatsReport {
    pub rows: Vec<FormatRow>,
    pub y416_layout: Vec<Y416Layout>,
}

const REQUESTS: &[u32] = &[
    pixbuf::Y416,
    pixbuf::X422,
    pixbuf::X420,
    pixbuf::XF20,
    pixbuf::V420,
    pixbuf::F420,
    pixbuf::RGBA_HALF,
    pixbuf::RGBA64,
    pixbuf::B64A,
    pixbuf::BGRA,
];

pub fn run(m: &Material) -> Result<FormatsReport> {
    let sources: [(&str, &Path); 5] = [
        ("ProRes 4444 XQ", &m.prores4444xq),
        ("ProRes 4444 XQ + alpha", &m.prores4444xq_alpha),
        ("ProRes 422 HQ", &m.prores422hq),
        ("HEVC Main10 4:2:0", &m.hevc10),
        ("H.264 8-bit 4:2:0", &m.h264),
    ];
    let mut rows = vec![];
    let mut layouts = vec![];
    for (name, path) in sources {
        for &req in REQUESTS {
            let t0 = Instant::now();
            match vt::decode_file(path, req) {
                Ok(info) => {
                    let n = info.frames.len().max(1);
                    let f0 = &info.frames[0].buffer;
                    rows.push(FormatRow {
                        source: name.into(),
                        codec: cf::fourcc_str(info.codec),
                        requested: cf::fourcc_str(req),
                        ok: f0.format() == req,
                        delivered: Some(cf::fourcc_str(f0.format())),
                        hardware: info.hardware,
                        ms_per_frame: Some(t0.elapsed().as_secs_f64() * 1000.0 / n as f64),
                        attachments: f0.attachments(),
                        error: None,
                    });
                    if req == pixbuf::Y416 && name.starts_with("ProRes 4444") {
                        layouts.push(y416_layout(name, &info)?);
                    }
                }
                Err(e) => rows.push(FormatRow {
                    source: name.into(),
                    codec: String::new(),
                    requested: cf::fourcc_str(req),
                    ok: false,
                    delivered: None,
                    hardware: None,
                    ms_per_frame: None,
                    attachments: vec![],
                    error: Some(e.0),
                }),
            }
        }
    }
    Ok(FormatsReport { rows, y416_layout: layouts })
}

fn y416_layout(name: &str, info: &vt::DecodeInfo) -> Result<Y416Layout> {
    let mut low4 = 0u64;
    let mut repl = 0u64;
    let mut n = 0u64;
    let mut alow4 = 0u64;
    let mut an = 0u64;
    let (mut ymin, mut ymax, mut cmin, mut cmax) = (u16::MAX, 0u16, u16::MAX, 0u16);
    let mut alphas = std::collections::BTreeSet::new();
    for f in &info.frames {
        let (d, _, _) = f.buffer.read_u16(None)?;
        for px in d.chunks(4) {
            alphas.insert(px[0]);
            an += 1;
            if px[0] & 15 == 0 {
                alow4 += 1;
            }
            for (k, &v) in px[1..].iter().enumerate() {
                n += 1;
                if v & 15 == 0 {
                    low4 += 1;
                }
                if v == (v >> 4) * 16 + (v >> 12) {
                    repl += 1;
                }
                if k == 0 {
                    ymin = ymin.min(v);
                    ymax = ymax.max(v);
                } else {
                    cmin = cmin.min(v);
                    cmax = cmax.max(v);
                }
            }
        }
    }
    Ok(Y416Layout {
        source: name.into(),
        low4_zero_fraction: low4 as f64 / n as f64,
        bit_replicated_fraction: repl as f64 / n as f64,
        y_min: ymin,
        y_max: ymax,
        c_min: cmin,
        c_max: cmax,
        alpha_distinct: alphas.len(),
        alpha_low4_zero_fraction: alow4 as f64 / an as f64,
    })
}
