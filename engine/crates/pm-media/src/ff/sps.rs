//! The H.264 sequence parameter set facts FFmpeg's stream probe otherwise
//! learns by decoding the first picture: pixel format, VUI colour, chroma
//! location and sample aspect (ITU-T H.264 7.3.2.1.1 and E.1.1).
//!
//! `Input::open` reads them from an MP4/MOV `avcC` record and lets FFmpeg's
//! probe skip the picture decode (35 ms for 1080p, 110 ms for UHD, on one
//! thread). Streams this reader cannot vouch for (field or MBAFF coding,
//! picture-structure SEI, unknown profiles) keep the full probe.

/// What the full probe would report for a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpsFacts {
    pub chroma_format: u32,
    pub bit_depth: u32,
    /// VUI `video_signal_type`: Some(full range) when present.
    pub full_range: Option<bool>,
    /// VUI colour description (H.273 codes), when present.
    pub colour: Option<[u8; 3]>,
    /// VUI chroma sample location (top field), when VUI is present at all:
    /// FFmpeg reports left (0) for a VUI without the element.
    pub chroma_location: Option<u32>,
    pub sample_aspect: (i32, i32),
    pub width: i32,
    pub height: i32,
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.pos / 8)?;
        let b = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Some(u32::from(b))
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        (0..n).try_fold(0, |v, _| Some((v << 1) | self.bit()?))
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.bits(zeros)?)
    }

    fn se(&mut self) -> Option<i32> {
        let k = self.ue()?;
        Some(if k % 2 == 1 {
            k.div_ceil(2) as i32
        } else {
            -((k / 2) as i32)
        })
    }
}

/// The first SPS of an `avcC` record.
pub(crate) fn from_avcc(avcc: &[u8]) -> Option<SpsFacts> {
    if *avcc.first()? != 1 || avcc.get(5)? & 0x1f == 0 {
        return None;
    }
    let len = usize::from(u16::from_be_bytes([*avcc.get(6)?, *avcc.get(7)?]));
    let nal = avcc.get(8..8 + len)?;
    if nal.first()? & 0x1f != 7 {
        return None;
    }
    // Emulation prevention: 00 00 03 → 00 00.
    let mut rbsp = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in &nal[1..] {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        rbsp.push(b);
    }
    parse(&rbsp)
}

fn parse(rbsp: &[u8]) -> Option<SpsFacts> {
    let mut r = Bits { data: rbsp, pos: 0 };
    let profile = r.bits(8)?;
    r.bits(16)?; // constraint flags, level
    r.ue()?; // seq_parameter_set_id
    let (mut chroma_format, mut bit_depth) = (1, 8);
    if matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma_format = r.ue()?;
        if chroma_format > 3 {
            return None;
        }
        if chroma_format == 3 && r.bit()? == 1 {
            return None; // separate colour planes
        }
        bit_depth = r.ue()? + 8;
        if r.ue()? + 8 != bit_depth || bit_depth > 14 {
            return None;
        }
        r.bit()?; // qpprime_y_zero_transform_bypass
        if r.bit()? == 1 {
            for i in 0..if chroma_format == 3 { 12 } else { 8 } {
                if r.bit()? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    let (mut last, mut next) = (8i32, 8i32);
                    for _ in 0..size {
                        if next != 0 {
                            next = (last + r.se()? + 256) % 256;
                        }
                        if next != 0 {
                            last = next;
                        }
                    }
                }
            }
        }
    } else if profile != 66 && profile != 77 && profile != 88 {
        return None;
    }
    r.ue()?; // log2_max_frame_num_minus4
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.bit()?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? {
                r.se()?;
            }
        }
        2 => {}
        _ => return None,
    }
    r.ue()?; // max_num_ref_frames
    r.bit()?; // gaps_in_frame_num_value_allowed
    let mbs_w = r.ue()? + 1;
    let map_h = r.ue()? + 1;
    if r.bit()? == 0 {
        return None; // field or MBAFF coding: the decoder decides
    }
    r.bit()?; // direct_8x8_inference
    let (sub_w, sub_h) = match chroma_format {
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    };
    let (mut crop_w, mut crop_h) = (0, 0);
    if r.bit()? == 1 {
        let (left, right, top, bottom) = (r.ue()?, r.ue()?, r.ue()?, r.ue()?);
        crop_w = (left + right) * sub_w;
        crop_h = (top + bottom) * sub_h;
    }
    let width = (mbs_w * 16).checked_sub(crop_w)? as i32;
    let height = (map_h * 16).checked_sub(crop_h)? as i32;
    let mut facts = SpsFacts {
        chroma_format,
        bit_depth,
        full_range: None,
        colour: None,
        chroma_location: None,
        sample_aspect: (0, 1),
        width,
        height,
    };
    if r.bit()? == 0 {
        return Some(facts);
    }
    facts.chroma_location = Some(0);
    if r.bit()? == 1 {
        facts.sample_aspect = match r.bits(8)? {
            255 => (r.bits(16)? as i32, r.bits(16)? as i32),
            idc => *[
                (0, 1),
                (1, 1),
                (12, 11),
                (10, 11),
                (16, 11),
                (40, 33),
                (24, 11),
                (20, 11),
                (32, 11),
                (80, 33),
                (18, 11),
                (15, 11),
                (64, 33),
                (160, 99),
                (4, 3),
                (3, 2),
                (2, 1),
            ]
            .get(idc as usize)
            .unwrap_or(&(0, 1)),
        };
    }
    if r.bit()? == 1 {
        r.bit()?; // overscan_appropriate
    }
    if r.bit()? == 1 {
        r.bits(3)?; // video_format
        facts.full_range = Some(r.bit()? == 1);
        if r.bit()? == 1 {
            facts.colour = Some([r.bits(8)? as u8, r.bits(8)? as u8, r.bits(8)? as u8]);
        }
    }
    if r.bit()? == 1 {
        facts.chroma_location = Some(r.ue()?);
        r.ue()?;
    }
    if r.bit()? == 1 {
        r.bits(32)?;
        r.bits(32)?;
        r.bit()?; // timing
    }
    let nal_hrd = r.bit()? == 1;
    if nal_hrd {
        hrd(&mut r)?;
    }
    let vcl_hrd = r.bit()? == 1;
    if vcl_hrd {
        hrd(&mut r)?;
    }
    if nal_hrd || vcl_hrd {
        r.bit()?; // low_delay_hrd
    }
    if r.bit()? == 1 {
        return None; // pic_struct SEI decides each frame's fields
    }
    Some(facts)
}

fn hrd(r: &mut Bits<'_>) -> Option<()> {
    let count = r.ue()? + 1;
    r.bits(8)?;
    for _ in 0..count {
        r.ue()?;
        r.ue()?;
        r.bit()?;
    }
    r.bits(20)?;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_golomb_and_a_minimal_baseline_sps() {
        let mut r = Bits {
            data: &[0b1010_0110, 0b0011_1000],
            pos: 0,
        };
        assert_eq!(r.ue(), Some(0));
        assert_eq!(r.ue(), Some(1));
        assert_eq!(r.ue(), Some(2));
        assert_eq!(r.se(), Some(7)); // 0001110: ue 13
        // Baseline 320×240 (20×15 macroblocks), POC type 2, no VUI.
        let mut w = Writer::default();
        w.bits(66, 8);
        w.bits(0, 16);
        for v in [0, 0, 2, 1] {
            w.ue(v);
        }
        w.bits(0, 1);
        w.ue(19);
        w.ue(14);
        w.bits(1, 1); // frame_mbs_only
        w.bits(0, 3); // direct_8x8, cropping, VUI
        let facts = parse(&w.finish()).unwrap();
        assert_eq!((facts.width, facts.height), (320, 240));
        assert_eq!(facts.chroma_location, None);
        assert_eq!(facts.sample_aspect, (0, 1));
    }

    #[derive(Default)]
    struct Writer {
        bits: Vec<u8>,
    }

    impl Writer {
        fn bits(&mut self, v: u32, n: u32) {
            for i in (0..n).rev() {
                self.bits.push(((v >> i) & 1) as u8);
            }
        }
        fn ue(&mut self, v: u32) {
            let n = 32 - (v + 1).leading_zeros();
            self.bits(0, n - 1);
            self.bits(v + 1, n);
        }
        fn finish(mut self) -> Vec<u8> {
            self.bits.push(1);
            while !self.bits.len().is_multiple_of(8) {
                self.bits.push(0);
            }
            self.bits
                .chunks(8)
                .map(|c| c.iter().fold(0, |a, &b| (a << 1) | b))
                .collect()
        }
    }
}
