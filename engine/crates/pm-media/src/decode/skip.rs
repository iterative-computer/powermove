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
                (Framing::Length((c[21] & 3) as usize + 1), (c[21] >> 3) & 7)
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
