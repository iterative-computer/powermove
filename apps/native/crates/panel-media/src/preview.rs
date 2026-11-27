//! Decoder-independent thumbnail/waveform requests. Pixel presentation is host
//! work; a model must never manufacture signal data when analysis is missing.
use crate::{Asset, Error};

/// Tagged AssetsPanel.svelte:222: capture after a real decoded frame at 10% of
/// the source, capped at half a second. No Chromium readiness workaround.
pub fn poster_time(asset: &Asset) -> Option<f64> {
    (asset.kind == "video")
        .then_some(asset.duration)
        .flatten()
        .filter(|d| d.is_finite() && *d > 0.0)
        .map(|d| (d * 0.1).min(0.5))
}

/// Mean of measured peaks in each requested bin (tag:254–263), normalized to
/// [0,1]. `None` means not analyzed; an absent waveform is never fake audio.
/// Bin count comes from the consumer, with a nonvisual allocation safety cap.
pub fn waveform_bins(peaks: Option<&[f64]>, bins: usize) -> Result<Option<Vec<f64>>, Error> {
    if bins == 0 || bins > 65_536 {
        return Err(Error::InvalidTarget);
    }
    let Some(peaks) = peaks.filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    if peaks
        .iter()
        .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
    {
        return Err(Error::InvalidMedia);
    }
    let mut out = Vec::with_capacity(bins);
    for b in 0..bins {
        let from = b * peaks.len() / bins;
        let to = ((b + 1) * peaks.len() / bins)
            .max(from + 1)
            .min(peaks.len());
        out.push(peaks[from..to].iter().sum::<f64>() / (to - from) as f64);
    }
    Ok(Some(out))
}
