//! Index for containers without frame tables (MPEG-TS, M2TS, MPEG-PS, VOB,
//! FLV, ASF, Ogg, elementary streams): FFmpeg's demuxer reads every packet;
//! nothing is decoded. Section 13: "scanning only where none exist".

use super::{CodecConfig, FrameIndex, IndexSource, IndexStats, Loc, NalCodec, Sample};
use crate::error::{MediaError, Result};
use crate::sys::ffmpeg as ff;
use std::path::Path;

/// H.264 or HEVC, for NAL-type marking.
pub fn nal_codec(id: ff::AVCodecID) -> Option<NalCodec> {
    match id {
        ff::AV_CODEC_ID_H264 => Some(NalCodec::H264),
        ff::AV_CODEC_ID_HEVC => Some(NalCodec::Hevc),
        _ => None,
    }
}

pub fn build(path: &Path, stream: usize) -> Result<FrameIndex> {
    let (pkts, tb, codec) = crate::ff::demux_scan(path, stream)?;
    if pkts.is_empty() {
        return Err(MediaError::invalid("the stream has no packets"));
    }
    // Packets are addressed by byte position plus their ordinal among the
    // packets reported at that position (Ogg reports page positions).
    let mut nths: Vec<u16> = Vec::with_capacity(pkts.len());
    let mut seen = std::collections::HashSet::new();
    for (k, p) in pkts.iter().enumerate() {
        let nth = match k.checked_sub(1).map(|j| (pkts[j].pos, nths[j])) {
            Some((pos, n)) if pos == p.pos => n.checked_add(1).ok_or_else(|| MediaError::unsupported("too many packets at one byte position"))?,
            _ => 0,
        };
        if p.pos < 0 || !seen.insert((p.pos, nth)) {
            return Err(MediaError::unsupported(
                "packet byte positions are missing or repeat; random access cannot address packets",
            ));
        }
        nths.push(nth);
    }
    // A missing PTS takes the DTS, and a stream without either is not
    // indexable. On a codec that reorders, DTS order is not presentation
    // order: presentation order then comes from a decode pass.
    let missing_pts = pkts.iter().any(|p| p.pts.is_none());
    let reorders = unsafe {
        let d = ff::avcodec_descriptor_get(codec);
        d.is_null() || (*d).props & ff::AV_CODEC_PROP_REORDER as i32 != 0
    };
    let mut samples: Vec<Sample> = pkts
        .iter()
        .zip(&nths)
        .map(|(p, &nth)| {
            let cts = p.pts.or(p.dts).ok_or_else(|| MediaError::unsupported("packets without timestamps"))?;
            Ok(Sample {
                dts: p.dts,
                cts,
                duration: p.duration,
                key: p.key,
                loc: Loc::Demux { pos: p.pos, nth, size: u32::try_from(p.size).unwrap_or(u32::MAX) },
                nal_type: p.nal_type,
            })
        })
        .collect::<Result<_>>()?;
    let mut hidden = vec![false; samples.len()];
    let mut source = IndexSource::DemuxScan;
    let mut detail = None;
    if missing_pts && reorders {
        let order = crate::ff::decode_order(path, stream, 4)?;
        presentation_from_order(&mut samples, &order, &mut hidden)?;
        source = IndexSource::DemuxScanDecodeOrder;
        detail = Some(format!(
            "no presentation timestamps on a reordering codec: presentation order from a decode pass ({} of {} packets presented)",
            order.len(),
            samples.len()
        ));
    }
    let nal = if pkts.iter().any(|p| p.nal_type.is_some()) { nal_codec(codec) } else { None };
    let mut idx = FrameIndex {
        source,
        track: stream as u64,
        timebase: tb,
        samples,
        frames: vec![],
        config: CodecConfig::default(),
        nal_codec: nal,
        stats: IndexStats { scan_reads: pkts.len(), detail, ..Default::default() },
    };
    // Packets the demuxer marks discardable (edit lists in MP4 read through
    // FFmpeg) are decoded but not presented, and so are packets a decode
    // pass never saw output.
    idx.frames = idx
        .samples
        .iter()
        .enumerate()
        .filter(|(i, _)| !pkts[*i].discard && !hidden[*i])
        .map(|(i, s)| super::Frame { sample: i, pts: s.cts })
        .collect();
    idx.finish();
    idx.summarize_open_gop();
    Ok(idx)
}

/// Assigns composition timestamps from a decode pass: the decode
/// timestamps, sorted, are the presentation slots, and the k-th picture the
/// decoder outputs takes slot k. Samples without output are hidden and take
/// the remaining slots so every composition timestamp stays unique.
fn presentation_from_order(samples: &mut [Sample], order: &[usize], hidden: &mut [bool]) -> Result<()> {
    let mut slots: Vec<i64> = samples
        .iter()
        .map(|s| s.dts.ok_or_else(|| MediaError::unsupported("packets without decode timestamps on a reordering codec")))
        .collect::<Result<_>>()?;
    slots.sort_unstable();
    if slots.windows(2).any(|w| w[0] == w[1]) {
        return Err(MediaError::unsupported("repeated decode timestamps on a reordering codec without presentation timestamps"));
    }
    let mut placed = vec![false; samples.len()];
    for &i in order {
        if i >= samples.len() || placed[i] {
            return Err(MediaError::decode(format!("the decode pass output packet {i} twice or out of range")));
        }
        placed[i] = true;
    }
    let mut free = slots.into_iter();
    for &i in order {
        samples[i].cts = free.next().expect("one slot per sample");
    }
    for (i, s) in samples.iter_mut().enumerate() {
        if !placed[i] {
            hidden[i] = true;
            s.cts = free.next().expect("one slot per sample");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(dts: i64) -> Sample {
        Sample { dts: Some(dts), cts: dts, duration: 1, key: false, loc: Loc::Demux { pos: dts, nth: 0, size: 1 }, nal_type: None }
    }

    #[test]
    fn decode_pass_order_becomes_presentation_order() {
        // I0 P3 B1 B2 in decode order; the decoder outputs I0 B1 B2 P3.
        let mut v = vec![s(0), s(1), s(2), s(3)];
        let mut hidden = vec![false; 4];
        presentation_from_order(&mut v, &[0, 2, 3, 1], &mut hidden).unwrap();
        assert_eq!(v.iter().map(|s| s.cts).collect::<Vec<_>>(), vec![0, 3, 1, 2]);
        assert_eq!(hidden, vec![false; 4]);
        // A leading picture the decoder drops is hidden and keeps a unique slot.
        let mut v = vec![s(10), s(11), s(12)];
        let mut hidden = vec![false; 3];
        presentation_from_order(&mut v, &[0, 2], &mut hidden).unwrap();
        assert_eq!(v.iter().map(|s| s.cts).collect::<Vec<_>>(), vec![10, 12, 11]);
        assert_eq!(hidden, vec![false, true, false]);
        // Output twice is an error, not a silent reorder.
        assert!(presentation_from_order(&mut [s(0), s(1)], &[0, 0], &mut [false; 2]).is_err());
    }
}
