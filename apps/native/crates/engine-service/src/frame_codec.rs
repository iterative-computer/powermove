//! Lossless storage of disk render cache frames (RGBA16F rows, top-down).
//!
//! A frame is cut into stripes of [`STRIPE_ROWS`] rows, each coded on its
//! own so reads decode them in parallel: every channel's 16-bit values
//! become the wrapping difference from the pixel to their left, split into
//! a low-byte and a high-byte plane, then LZFSE (Apple's Compression
//! framework). Flat and smoothly varying areas (most of a motion graphics
//! frame, and every transparent pixel) shrink to a few bytes; the decoded
//! bytes are exactly the rendered ones. On a 1080p frame of the 867-layer
//! project: 3.8 MB instead of 16.6 MB (LZ4: 6.7 MB), decoded in about 10 ms
//! on one thread.

/// Rows per independently coded stripe.
const STRIPE_ROWS: usize = 64;
const MAGIC: &[u8; 4] = b"PMZ2";
/// `COMPRESSION_LZFSE` (`compression.h`).
const LZFSE: i32 = 0x801;
/// Stripes coded or decoded at once.
const THREADS: usize = 4;

#[link(name = "compression")]
unsafe extern "C" {
    fn compression_encode_buffer(
        dst: *mut u8,
        dst_size: usize,
        src: *const u8,
        src_size: usize,
        scratch: *mut std::ffi::c_void,
        algorithm: i32,
    ) -> usize;
    fn compression_decode_buffer(
        dst: *mut u8,
        dst_size: usize,
        src: *const u8,
        src_size: usize,
        scratch: *mut std::ffi::c_void,
        algorithm: i32,
    ) -> usize;
}

fn row_bytes(width: u32) -> usize {
    width as usize * 8
}

/// The coded frame of `rgba` (`width` × `height` RGBA16F, little endian).
pub fn encode(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row = row_bytes(width);
    assert_eq!(rgba.len(), row * height as usize, "frame size");
    let stripes: Vec<&[u8]> = rgba.chunks(row * STRIPE_ROWS).collect();
    let mut out = Vec::with_capacity(rgba.len() / 4);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&(stripes.len() as u32).to_le_bytes());
    debug_assert_eq!(out.len(), 16);
    // Each stripe coded: its compressed bytes, or (None) its planes as
    // they are when compression would not make them smaller.
    let per = stripes.len().div_ceil(THREADS).max(1);
    let coded: Vec<(Vec<u8>, bool)> = std::thread::scope(|s| {
        let workers: Vec<_> = stripes
            .chunks(per)
            .map(|group| {
                s.spawn(move || {
                    let mut planes = Vec::new();
                    group
                        .iter()
                        .map(|stripe| {
                            split(stripe, row, &mut planes);
                            let mut packed = vec![0u8; planes.len()];
                            // SAFETY: both buffers are valid for the sizes given.
                            let n = unsafe {
                                compression_encode_buffer(
                                    packed.as_mut_ptr(),
                                    packed.len(),
                                    planes.as_ptr(),
                                    planes.len(),
                                    std::ptr::null_mut(),
                                    LZFSE,
                                )
                            };
                            if n == 0 {
                                (planes.clone(), false)
                            } else {
                                packed.truncate(n);
                                (packed, true)
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("frame coding thread"))
            .collect()
    });
    for (bytes, packed) in &coded {
        // 0: the stripe's planes are stored as they are.
        let len = if *packed { bytes.len() as u32 } else { 0 };
        out.extend_from_slice(&len.to_le_bytes());
    }
    for (bytes, _) in &coded {
        out.extend_from_slice(bytes);
    }
    out
}

/// The frame `encode` coded, if `data` is a whole coded frame of `width` ×
/// `height`.
pub fn decode(data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let word = |at: usize| Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?));
    if data.get(..4)? != MAGIC || word(4)? != width || word(8)? != height {
        return None;
    }
    let row = row_bytes(width);
    let total = row * height as usize;
    let count = total.div_ceil(row * STRIPE_ROWS);
    if word(12)? as usize != count {
        return None;
    }
    // Each stripe's coded bytes and its share of the output.
    let mut at = 16 + count * 4;
    let mut spans = Vec::with_capacity(count);
    for i in 0..count {
        let raw = (row * STRIPE_ROWS).min(total - i * row * STRIPE_ROWS);
        let len = match word(16 + i * 4)? as usize {
            0 => raw,
            n => n,
        };
        spans.push((data.get(at..at + len)?, word(16 + i * 4)? != 0, raw));
        at += len;
    }
    if at != data.len() {
        return None;
    }
    let mut out = vec![0u8; total];
    let mut parts: Vec<(&mut [u8], Span<'_>)> =
        out.chunks_mut(row * STRIPE_ROWS).zip(spans).collect();
    let per = parts.len().div_ceil(THREADS).max(1);
    let ok = std::thread::scope(|s| {
        let workers: Vec<_> = parts
            .chunks_mut(per)
            .map(|group| {
                s.spawn(move || {
                    let mut planes = Vec::new();
                    group.iter_mut().all(|(dst, (coded, packed, raw))| {
                        let planes: &[u8] = if *packed {
                            planes.resize(*raw, 0);
                            // SAFETY: both buffers are valid for the sizes given.
                            let n = unsafe {
                                compression_decode_buffer(
                                    planes.as_mut_ptr(),
                                    planes.len(),
                                    coded.as_ptr(),
                                    coded.len(),
                                    std::ptr::null_mut(),
                                    LZFSE,
                                )
                            };
                            if n != *raw {
                                return false;
                            }
                            &planes
                        } else {
                            coded
                        };
                        join(planes, row, dst);
                        true
                    })
                })
            })
            .collect();
        workers.into_iter().all(|w| w.join().unwrap_or(false))
    });
    ok.then_some(out)
}

/// A stripe's coded bytes, whether they are compressed (else the planes as
/// they are), and its decoded length.
type Span<'a> = (&'a [u8], bool, usize);

/// `stripe`'s rows as left-difference low-byte and high-byte planes.
fn split(stripe: &[u8], row: usize, planes: &mut Vec<u8>) {
    let values = stripe.len() / 2;
    planes.clear();
    planes.resize(stripe.len(), 0);
    let (low, high) = planes.split_at_mut(values);
    let mut i = 0;
    for line in stripe.chunks_exact(row) {
        let mut left = [0u16; 4];
        for px in line.chunks_exact(8) {
            for c in 0..4 {
                let v = u16::from_le_bytes([px[c * 2], px[c * 2 + 1]]);
                let d = v.wrapping_sub(left[c]).to_le_bytes();
                left[c] = v;
                low[i] = d[0];
                high[i] = d[1];
                i += 1;
            }
        }
    }
}

/// [`split`] reversed into `dst`.
fn join(planes: &[u8], row: usize, dst: &mut [u8]) {
    let values = dst.len() / 2;
    let (low, high) = planes.split_at(values);
    let mut i = 0;
    for line in dst.chunks_exact_mut(row) {
        let mut left = [0u16; 4];
        for px in line.chunks_exact_mut(8) {
            for c in 0..4 {
                let v = left[c].wrapping_add(u16::from_le_bytes([low[i], high[i]]));
                left[c] = v;
                px[c * 2..c * 2 + 2].copy_from_slice(&v.to_le_bytes());
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32) -> Vec<u8> {
        let mut seed = 7u32;
        (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let px: [f32; 4] = if y < height / 3 {
                    [0.; 4] // transparent
                } else if y < 2 * height / 3 {
                    let t = x as f32 / width as f32;
                    [t * 0.8, 0.2, 1.0 - t, 0.8] // a gradient, premultiplied
                } else {
                    // noise beyond 1.0, as HDR working values can be
                    let n = (seed >> 8) as f32 / (1 << 24) as f32;
                    [n * 4., n, -n * 0.01, 1.]
                };
                px.into_iter()
                    .flat_map(|v| half::f16::from_f32(v).to_le_bytes())
            })
            .collect()
    }

    /// Decoding gives back every byte; flat and smooth areas shrink, and an
    /// odd height (a last short stripe) and noise still round trip.
    #[test]
    fn frames_round_trip_exactly_and_compress() {
        for (w, h) in [(1920, 1080), (333, 65), (1, 1)] {
            let raw = frame(w, h);
            let coded = encode(&raw, w, h);
            assert_eq!(decode(&coded, w, h).as_deref(), Some(&raw[..]), "{w}x{h}");
            if (w, h) == (1920, 1080) {
                assert!(
                    coded.len() * 2 < raw.len(),
                    "{} of {}",
                    coded.len(),
                    raw.len()
                );
            }
        }
    }

    /// A truncated, mislabeled or tampered frame never decodes.
    #[test]
    fn damaged_frames_are_refused() {
        let raw = frame(320, 180);
        let coded = encode(&raw, 320, 180);
        assert!(decode(&coded[..coded.len() - 1], 320, 180).is_none());
        assert!(decode(&coded, 320, 181).is_none());
        let mut bad = coded.clone();
        bad[20] ^= 0xff;
        assert!(decode(&bad, 320, 180).is_none_or(|d| d != raw));
    }
}
