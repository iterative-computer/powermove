//! Visible affine image tiles. Geometry stays on the original quad's pixel
//! grid; only its source rectangle is divided, so a crop cannot move edges.

use pm_media::image::mip_cache::VisibleTile;

/// Conservative source-UV bounds of the output rectangle. A one-output-pixel
/// margin covers bilinear footprints along the clipped viewer boundary.
pub(crate) fn visible_uv_bounds(
    matrix: [f64; 6],
    uv: [f64; 4],
    output: [f64; 4],
) -> Option<[f64; 4]> {
    if !matrix.iter().chain(uv.iter()).chain(output.iter()).all(|v| v.is_finite())
        || uv[2] <= 0.0 || uv[3] <= 0.0
    { return None; }
    let [a, b, c, d, e, f] = matrix;
    let det = a * d - b * c;
    if det.abs() < 1e-12 { return None; }
    let [x, y, w, h] = output;
    if w <= 0.0 || h <= 0.0 { return None; }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for px in [x - 1.0, x + w + 1.0] {
        for py in [y - 1.0, y + h + 1.0] {
            let dx = px - e; let dy = py - f;
            let u = (d * dx - c * dy) / det;
            let v = (-b * dx + a * dy) / det;
            let source = [uv[0] + u * uv[2], uv[1] + v * uv[3]];
            for axis in 0..2 {
                lo[axis] = lo[axis].min(source[axis]);
                hi[axis] = hi[axis].max(source[axis]);
            }
        }
    }
    Some([lo[0], lo[1], hi[0], hi[1]])
}

/// A tile's unique interior as a subquad and its padded texture window.
/// Neighbor texels exist in the gutter, but no interior is drawn twice.
pub(crate) fn quad(
    matrix: [f64; 6],
    uv: [f64; 4],
    tile: &VisibleTile,
) -> Option<([f64; 6], [f64; 4])> {
    if uv[2] <= 0.0 || uv[3] <= 0.0 { return None; }
    let (fw, fh) = tile.full_size;
    let (x, y) = tile.origin;
    let (w, h) = tile.content_size;
    let sx0 = (x as f64 / fw as f64).max(uv[0]);
    let sy0 = (y as f64 / fh as f64).max(uv[1]);
    let sx1 = ((x + w) as f64 / fw as f64).min(uv[0] + uv[2]);
    let sy1 = ((y + h) as f64 / fh as f64).min(uv[1] + uv[3]);
    if sx1 <= sx0 || sy1 <= sy0 { return None; }
    let u0 = (sx0 - uv[0]) / uv[2];
    let v0 = (sy0 - uv[1]) / uv[3];
    let du = (sx1 - sx0) / uv[2];
    let dv = (sy1 - sy0) / uv[3];
    let [a, b, c, d, e, f] = matrix;
    let sub = [a * du, b * du, c * dv, d * dv, a * u0 + c * v0 + e, b * u0 + d * v0 + f];
    let tw = tile.image.frame.format.width as f64;
    let th = tile.image.frame.format.height as f64;
    let texture_x = x - tile.content_offset.0;
    let texture_y = y - tile.content_offset.1;
    let texture_uv = [
        (sx0 * fw as f64 - texture_x as f64) / tw,
        (sy0 * fh as f64 - texture_y as f64) / th,
        (sx1 - sx0) * fw as f64 / tw,
        (sy1 - sy0) * fh as f64 / th,
    ];
    Some((sub, texture_uv))
}

/// Playback's level-0 trilinear sampling is the ordinary bilinear sampler
/// when no output direction shrinks source pixels. The tile path uses that
/// equivalent sampler; minified or Final draws keep the whole source's mip
/// pyramid and analytic edges.
pub(crate) fn playback_bilinear(matrix: [f64; 6], uv: [f64; 4], size: (usize, usize)) -> bool {
    let [a, b, c, d, _, _] = matrix;
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-12 { return false; }
    let sx = size.0 as f64 * uv[2] / det;
    let sy = size.1 as f64 * uv[3] / det;
    let (j00, j01, j10, j11) = (sx * d, -sx * c, -sy * b, sy * a);
    let aa = j00 * j00 + j10 * j10;
    let bb = j00 * j01 + j10 * j11;
    let cc = j01 * j01 + j11 * j11;
    let largest = (aa + cc + ((aa - cc).powi(2) + 4.0 * bb * bb).sqrt()) / 2.0;
    largest.is_finite() && largest <= 1.0 + 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_affine_bounds_rotated_crop() {
        // 90-degree rotation: source u follows output y, v follows -x.
        let bounds = visible_uv_bounds([0.0, 2.0, -2.0, 0.0, 10.0, 20.0],
            [0.0, 0.0, 1.0, 1.0], [8.0, 20.0, 2.0, 2.0]).unwrap();
        assert!(bounds[0] <= -0.5 && bounds[2] >= 1.5);
        assert!(bounds[1] <= -0.5 && bounds[3] >= 1.5);
    }

    #[test]
    fn playback_base_level_only_when_not_minified() {
        assert!(playback_bilinear([256.0, 0.0, 0.0, 256.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 1.0], (256, 256)));
        assert!(!playback_bilinear([128.0, 0.0, 0.0, 128.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 1.0], (256, 256)));
    }
}
