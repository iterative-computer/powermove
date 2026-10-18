//! Pictures a seek may leave out (docs/engine/scrub.md).
//!
//! To show frame N of a long GOP the decoder runs from the GOP's sync
//! sample through N in decode order. Pictures no other picture references
//! need not be decoded on the way unless they are shown: H.264 pictures
//! whose every VCL NAL unit has `nal_ref_idc` 0, and HEVC sub-layer
//! non-reference pictures (`TRAIL_N`, `TSA_N`, `STSA_N`, `RADL_N`, `RASL_N`,
//! reserved `RSV_VCL_N*`) in the highest temporal sub-layer. With x264's
//! default three B frames and a B pyramid half the pictures of a GOP are
//! such pictures. Leaving them out is exact: no decoded picture changes, only
//! pictures nobody asked for are not made (AVFoundation drops the same
//! pictures when it scrubs).

use crate::index::{FrameIndex, NalCodec};

/// How NAL units are framed inside a sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Framing {
    /// Each NAL unit after a big-endian length of this many bytes (`avcC`,
    /// `hvcC`).
    Length(usize),
    /// Annex B start codes.
    AnnexB,
}

/// Classifies samples of one H.264 or HEVC stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disposable {
    codec: NalCodec,
    framing: Framing,
    /// HEVC: the number of temporal sub-layers (`hvcC` numTemporalLayers);
    /// only pictures in the highest one are unreferenced.
    hevc_layers: u8,
}

impl Disposable {
    /// The classifier for an index whose samples carry H.264 or HEVC, or
    /// `None` when the stream is something else or its framing is unknown
    /// (then nothing is left out).
    pub fn for_index(idx: &FrameIndex) -> Option<Disposable> {
        let codec = idx.nal_codec.or_else(|| {
            if idx.config.atom(b"avcC").is_some() {
                Some(NalCodec::H264)
            } else if idx.config.atom(b"hvcC").is_some() {
                Some(NalCodec::Hevc)
            } else {
                match idx.config.codec_id.as_deref() {
                    Some("V_MPEG4/ISO/AVC") => Some(NalCodec::H264),
                    Some("V_MPEGH/ISO/HEVC") => Some(NalCodec::Hevc),
                    _ => None,
                }
            }
        })?;
        let record = match codec {
            NalCodec::H264 => idx.config.atom(b"avcC"),
            NalCodec::Hevc => idx.config.atom(b"hvcC"),
        }
        .or(idx.config.codec_private.as_deref());
        let (framing, hevc_layers) = match (codec, record) {
            (NalCodec::H264, Some(c)) if c.len() > 4 && c[0] == 1 => {
                (Framing::Length((c[4] & 3) as usize + 1), 0)
            }
            (NalCodec::Hevc, Some(c)) if c.len() > 22 && c[0] == 1 => {
                (Framing::Length((c[21] & 3) as usize + 1), hevc_layers(c))
            }
            // No decoder configuration record: parameter sets travel in band.
            (NalCodec::H264, None) => (Framing::AnnexB, 0),
            _ => return None,
        };
        if framing == Framing::Length(3) {
            // Not a valid NAL length size.
            return None;
        }
        Some(Disposable {
            codec,
            framing,
            hevc_layers,
        })
    }

    /// Whether a sample (one access unit) is a picture nothing references.
    /// Unparsable samples are never disposable.
    pub fn disposable(&self, sample: &[u8]) -> bool {
        let mut vcl = 0usize;
        let mut all = true;
        let mut visit = |nal: &[u8]| {
            let Some(&b0) = nal.first() else { return };
            match self.codec {
                NalCodec::H264 => {
                    let t = b0 & 0x1F;
                    if (1..=5).contains(&t) {
                        vcl += 1;
                        all &= t != 5 && (b0 >> 5) & 3 == 0;
                    }
                }
                NalCodec::Hevc => {
                    let t = (b0 >> 1) & 0x3F;
                    if t < 32 {
                        vcl += 1;
                        let tid = nal.get(1).map_or(0, |b| (b & 7).saturating_sub(1));
                        let sub_layer_non_reference = t <= 14 && t % 2 == 0;
                        let top = self.hevc_layers > 0 && tid + 1 == self.hevc_layers;
                        all &= sub_layer_non_reference && top;
                    }
                }
            }
        };
        let complete = match self.framing {
            Framing::Length(n) => for_each_length_prefixed(sample, n, &mut visit),
            Framing::AnnexB => {
                for nal in crate::vt::annexb_nals(sample) {
                    visit(nal);
                }
                true
            }
        };
        complete && vcl > 0 && all
    }
}

/// The temporal sub-layers of an HEVC stream from its `hvcC` record, when
/// the record's numTemporalLayers agrees with every SPS it carries
/// (`sps_max_sub_layers_minus1 + 1`, the bound on every picture's
/// TemporalId); otherwise 0 (unknown: nothing is left out). A record that
/// under-reports its sub-layers would make referenced pictures look like
/// top-layer ones.
fn hevc_layers(c: &[u8]) -> u8 {
    let layers = (c[21] >> 3) & 7;
    let mut sps = 0usize;
    let mut p = 23usize;
    for _ in 0..c[22] {
        let Some(h) = c.get(p..p + 3) else { return 0 };
        let (kind, count) = (h[0], u16::from_be_bytes([h[1], h[2]]));
        p += 3;
        for _ in 0..count {
            let Some(l) = c.get(p..p + 2) else { return 0 };
            let len = u16::from_be_bytes([l[0], l[1]]) as usize;
            p += 2;
            let Some(nal) = c.get(p..p + len) else {
                return 0;
            };
            p += len;
            if kind & 0x3F == 33 {
                // The byte after the NAL header: sps_video_parameter_set_id
                // (4 bits), sps_max_sub_layers_minus1 (3), nesting (1).
                let Some(&b) = nal.get(2) else { return 0 };
                if (b >> 1) & 7 != layers.wrapping_sub(1) {
                    return 0;
                }
                sps += 1;
            }
        }
    }
    if sps == 0 { 0 } else { layers }
}

/// Calls `f` with each NAL unit; false when the sample is malformed.
fn for_each_length_prefixed(b: &[u8], n: usize, f: &mut impl FnMut(&[u8])) -> bool {
    let mut p = 0usize;
    while p < b.len() {
        let Some(h) = b.get(p..p + n) else {
            return false;
        };
        let len = h.iter().fold(0usize, |a, &x| (a << 8) | x as usize);
        p += n;
        let Some(nal) = b.get(p..p + len) else {
            return false;
        };
        f(nal);
        p += len;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h264() -> Disposable {
        Disposable {
            codec: NalCodec::H264,
            framing: Framing::Length(4),
            hevc_layers: 0,
        }
    }

    fn sample(nals: &[&[u8]]) -> Vec<u8> {
        let mut v = vec![];
        for n in nals {
            v.extend_from_slice(&(n.len() as u32).to_be_bytes());
            v.extend_from_slice(n);
        }
        v
    }

    #[test]
    fn h264_non_reference_slices_are_disposable() {
        let d = h264();
        // SEI then a non-reference slice (nal_ref_idc 0, type 1).
        assert!(d.disposable(&sample(&[&[0x06, 0x05], &[0x01, 0x9a]])));
        // A reference B slice (nal_ref_idc 2).
        assert!(!d.disposable(&sample(&[&[0x41, 0x9a]])));
        // IDR is never disposable.
        assert!(!d.disposable(&sample(&[&[0x05, 0x88]])));
        // Two fields: the second is a reference picture.
        assert!(!d.disposable(&sample(&[&[0x01, 0x9a], &[0x21, 0x9a]])));
        // No slice at all, or a truncated sample.
        assert!(!d.disposable(&sample(&[&[0x06, 0x05]])));
        let mut cut = sample(&[&[0x01, 0x9a, 0x00]]);
        cut.pop();
        assert!(!d.disposable(&cut));
    }

    #[test]
    fn hevc_only_top_sub_layer_non_reference_pictures_are_disposable() {
        let one = Disposable {
            codec: NalCodec::Hevc,
            framing: Framing::Length(4),
            hevc_layers: 1,
        };
        // TRAIL_N (0), temporal id 0.
        assert!(one.disposable(&sample(&[&[0x00, 0x01, 0xaf]])));
        // TRAIL_R (1).
        assert!(!one.disposable(&sample(&[&[0x02, 0x01, 0xaf]])));
        // RASL_N (8).
        assert!(one.disposable(&sample(&[&[0x10, 0x01, 0xaf]])));
        // Two sub-layers: TRAIL_N in layer 0 may be referenced by layer 1.
        let two = Disposable {
            hevc_layers: 2,
            ..one
        };
        assert!(!two.disposable(&sample(&[&[0x00, 0x01, 0xaf]])));
        assert!(two.disposable(&sample(&[&[0x00, 0x02, 0xaf]])));
        // Unknown sub-layer count: nothing is left out.
        let unknown = Disposable {
            hevc_layers: 0,
            ..one
        };
        assert!(!unknown.disposable(&sample(&[&[0x00, 0x01, 0xaf]])));
    }

    /// An `hvcC` with numTemporalLayers `layers` and one SPS per entry of
    /// `sps` (its sps_max_sub_layers_minus1 + 1).
    fn hvcc(layers: u8, sps: &[u8]) -> Vec<u8> {
        let mut c = vec![0u8; 23];
        c[0] = 1;
        c[21] = (layers << 3) | 3;
        c[22] = 2;
        // A VPS array, then the SPS array.
        c.extend_from_slice(&[0x80 | 32, 0, 1, 0, 4, 0x40, 0x01, 0x0c, 0x01]);
        c.extend_from_slice(&[0x80 | 33]);
        c.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        for &m in sps {
            c.extend_from_slice(&[0, 4, 0x42, 0x01, ((m - 1) << 1) | 1, 0x01]);
        }
        c
    }

    #[test]
    fn hevc_sub_layers_must_agree_with_the_sps() {
        assert_eq!(hevc_layers(&hvcc(1, &[1])), 1);
        assert_eq!(hevc_layers(&hvcc(3, &[3, 3])), 3);
        // The record under-reports (TemporalId 1 pictures may exist):
        // TRAIL_N pictures at TemporalId 0 may be referenced.
        assert_eq!(hevc_layers(&hvcc(1, &[2])), 0);
        assert_eq!(hevc_layers(&hvcc(2, &[2, 1])), 0);
        // No SPS in the record (parameter sets in band), or a cut record.
        assert_eq!(hevc_layers(&hvcc(1, &[])), 0);
        let mut cut = hvcc(1, &[1]);
        cut.truncate(cut.len() - 2);
        assert_eq!(hevc_layers(&cut), 0);
    }

    #[test]
    fn annex_b_samples_are_parsed() {
        let d = Disposable {
            framing: Framing::AnnexB,
            ..h264()
        };
        assert!(d.disposable(&[0, 0, 0, 1, 0x09, 0xf0, 0, 0, 1, 0x01, 0x9a]));
        assert!(!d.disposable(&[0, 0, 0, 1, 0x09, 0xf0, 0, 0, 1, 0x61, 0x9a]));
    }
}
