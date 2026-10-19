//! SkScan_AAAPath.cpp's convex edge walk, chrome/m152. See the parent
//! module for Skia's BSD-3-Clause license and fixed-point primitives.
use super::*;

fn update(e: &mut Edge) -> bool {
    if e.curve_count < 0 {
        e.update_cubic()
    } else if e.curve_count > 0 {
        e.update_quadratic()
    } else {
        false
    }
}

fn smooth(e: &Edge, next: &Edge) -> bool {
    if e.curve_count < 0 {
        let dd = i32::from(e.curve_shift);
        (e.cdx.abs() >> 1) >= (e.cddx.abs() >> dd)
            && (e.cdy.abs() >> 1) >= (e.cddy.abs() >> dd)
            && ((e.cdy - (e.cddy >> dd)) >> e.cubic_dshift) >= FIXED1
    } else if e.curve_count > 0 {
        (e.qdx.abs() >> 1) >= e.qddx.abs()
            && (e.qdy.abs() >> 1) >= e.qddy.abs()
            && ((e.qdy - e.qddy) >> e.curve_shift) >= FIXED1
    } else {
        next.dx.saturating_sub(e.dx).saturating_abs() <= FIXED1
            && next.lower_y - next.upper_y >= FIXED1
    }
}

fn smooth_pair(edges: &Edges, l: usize, r: usize, mut curr: usize, stop: i32) -> bool {
    let e = &edges.0;
    if e[curr].upper_y >= int_to_fixed(stop) {
        return false;
    }
    if e[l].lower_y + FIXED1 < e[r].lower_y {
        return smooth(&e[l], &e[curr]);
    }
    if e[l].lower_y > e[r].lower_y + FIXED1 {
        return smooth(&e[r], &e[curr]);
    }
    let mut next = e[curr].next;
    if e[next].upper_y >= int_to_fixed(stop) {
        return false;
    }
    if e[next].upper_x < e[curr].upper_x {
        std::mem::swap(&mut curr, &mut next);
    }
    smooth(&e[l], &e[curr]) && smooth(&e[r], &e[next])
}

impl Ctx<'_> {
    fn convex_add(&mut self, x: i32, y: i32, n: i32, a: u8, mask: bool) {
        if mask {
            let row = self.add.mask_row(y).unwrap();
            for dx in 0..n {
                self.add.mask_add(row, x + dx, a);
            }
        } else {
            self.add.blit_anti_hn(x, y, n, a, self.dev);
        }
    }
    fn convex_v(&mut self, x: i32, y: i32, h: i32, a: u8, mask: bool) {
        if a == 0 {
            return;
        }
        if mask {
            for yy in y..y + h {
                let row = self.add.mask_row(yy).unwrap();
                self.add.mask_set(row, x, a);
            }
        } else {
            self.dev.blit_v(x, y, h, a);
        }
    }
    fn convex_flush(&mut self, y: Fixed, next: Fixed, mask: bool) {
        if !mask && fixed_floor_to_int(y) != fixed_floor_to_int(next) {
            self.add.flush(self.dev);
        }
    }
}

pub(super) fn walk(
    edges: &mut Edges,
    ctx: &mut Ctx<'_>,
    stop: i32,
    left_bound: Fixed,
    right_bound: Fixed,
    mask: bool,
) {
    let mut l = edges.0[HEAD].next;
    let mut r = edges.0[l].next;
    let mut curr = edges.0[r].next;
    let mut y = edges.0[l].upper_y.max(edges.0[r].upper_y);
    loop {
        while edges.0[l].lower_y <= y {
            if !update(&mut edges.0[l]) {
                if fixed_floor_to_int(edges.0[curr].upper_y) >= stop {
                    return;
                }
                l = curr;
                curr = edges.0[curr].next;
            }
        }
        while edges.0[r].lower_y <= y {
            if !update(&mut edges.0[r]) {
                if fixed_floor_to_int(edges.0[curr].upper_y) >= stop {
                    return;
                }
                r = curr;
                curr = edges.0[curr].next;
            }
        }
        if fixed_floor_to_int(y) >= stop {
            break;
        }
        edges.0[l].go_y(y);
        edges.0[r].go_y(y);
        if edges.0[l].x > edges.0[r].x
            || (edges.0[l].x == edges.0[r].x && edges.0[l].dx > edges.0[r].dx)
        {
            std::mem::swap(&mut l, &mut r);
        }
        let mut bottom = edges.0[l].lower_y.min(edges.0[r].lower_y);
        if smooth_pair(edges, l, r, curr, stop) {
            bottom = fixed_ceil_to_fixed(bottom);
        }
        bottom = bottom.min(int_to_fixed(stop));
        let (mut left, mut right) = (left_bound.max(edges.0[l].x), right_bound.min(edges.0[r].x));
        let (dl, dr) = (edges.0[l].dx, edges.0[r].dx);
        if dl | dr == 0 {
            let (fl, fr) = (fixed_ceil_to_int(left), fixed_floor_to_int(right));
            let (pl, pr) = (int_to_fixed(fl) - left, right - int_to_fixed(fr));
            let (ft, fb) = (fixed_ceil_to_int(y), fixed_floor_to_int(bottom));
            let (mut pt, mut pb) = (int_to_fixed(ft) - y, bottom - int_to_fixed(fb));
            if ft > fb {
                pt -= FIXED1 - pb;
                pb = 0;
            }
            if fr >= fl {
                if pt > 0 {
                    if pl > 0 {
                        ctx.convex_add(fl - 1, ft - 1, 1, fixed_to_alpha(fixed_mul(pt, pl)), mask);
                    }
                    ctx.convex_add(fl, ft - 1, fr - fl, fixed_to_alpha(pt), mask);
                    if pr > 0 {
                        ctx.convex_add(fr, ft - 1, 1, fixed_to_alpha(fixed_mul(pt, pr)), mask);
                    }
                    ctx.convex_flush(y, y + pt, mask);
                }
                if fb > ft && (fr > fl || fixed_to_alpha(pl) > 0 || fixed_to_alpha(pr) > 0) {
                    ctx.convex_v(fl - 1, ft, fb - ft, fixed_to_alpha(pl), mask);
                    ctx.convex_v(fr, ft, fb - ft, fixed_to_alpha(pr), mask);
                    for yy in ft..fb {
                        if mask {
                            let row = ctx.add.mask_row(yy).unwrap();
                            for x in fl..fr {
                                ctx.add.mask_set(row, x, 255);
                            }
                        } else {
                            ctx.dev.blit_h(fl, yy, fr - fl);
                        }
                    }
                }
                if pb > 0 {
                    if pl > 0 {
                        ctx.convex_add(fl - 1, fb, 1, fixed_to_alpha(fixed_mul(pb, pl)), mask);
                    }
                    ctx.convex_add(fl, fb, fr - fl, fixed_to_alpha(pb), mask);
                    if pr > 0 {
                        ctx.convex_add(fr, fb, 1, fixed_to_alpha(fixed_mul(pb, pr)), mask);
                    }
                }
            } else {
                let width = right - left;
                if width > 0 {
                    if pt > 0 {
                        ctx.convex_add(
                            fl - 1,
                            ft - 1,
                            1,
                            fixed_to_alpha(fixed_mul(pt, width)),
                            mask,
                        );
                        ctx.convex_flush(y, y + pt, mask);
                    }
                    if fb > ft {
                        ctx.convex_v(fl - 1, ft, fb - ft, fixed_to_alpha(width), mask);
                    }
                    if pb > 0 {
                        ctx.convex_add(fl - 1, fb, 1, fixed_to_alpha(fixed_mul(pb, width)), mask);
                    }
                }
            }
            y = bottom;
        } else {
            const HALF: Fixed = FIXED1 >> 5;
            const MASK: Fixed = !((FIXED1 >> 4) - 1);
            left += HALF;
            right += HALF;
            let mut count = fixed_ceil_to_int(bottom) - fixed_floor_to_int(y);
            if count > 1 {
                if fixed_floor_to_fixed(y) != y {
                    count -= 1;
                    let next = fixed_ceil_to_fixed(y + 1);
                    let dy = next - y;
                    let (nl, nr) = (left + fixed_mul(dl, dy), right + fixed_mul(dr, dy));
                    let row = if mask {
                        ctx.add.mask_row(y >> 16)
                    } else {
                        None
                    };
                    ctx.trapezoid_row(
                        y >> 16,
                        left & MASK,
                        right & MASK,
                        nl & MASK,
                        nr & MASK,
                        edges.0[l].dy,
                        edges.0[r].dy,
                        partial_alpha_fixed(255, dy),
                        row,
                        false,
                    );
                    ctx.convex_flush(y, next, mask);
                    left = nl;
                    right = nr;
                    y = next;
                }
                while count > 1 {
                    count -= 1;
                    let (next, nl, nr) = (y + FIXED1, left + dl, right + dr);
                    let row = if mask {
                        ctx.add.mask_row(y >> 16)
                    } else {
                        None
                    };
                    ctx.trapezoid_row(
                        y >> 16,
                        left & MASK,
                        right & MASK,
                        nl & MASK,
                        nr & MASK,
                        edges.0[l].dy,
                        edges.0[r].dy,
                        255,
                        row,
                        false,
                    );
                    ctx.convex_flush(y, next, mask);
                    left = nl;
                    right = nr;
                    y = next;
                }
            }
            let dy = bottom - y;
            let nl = (left + fixed_mul(dl, dy)).max(left_bound + HALF);
            let nr = (right + fixed_mul(dr, dy)).min(right_bound + HALF);
            let row = if mask {
                ctx.add.mask_row(y >> 16)
            } else {
                None
            };
            ctx.trapezoid_row(
                y >> 16,
                left & MASK,
                right & MASK,
                nl & MASK,
                nr & MASK,
                edges.0[l].dy,
                edges.0[r].dy,
                partial_alpha_fixed(255, dy),
                row,
                false,
            );
            ctx.convex_flush(y, bottom, mask);
            left = nl - HALF;
            right = nr - HALF;
            y = bottom;
        }
        edges.0[l].x = left;
        edges.0[r].x = right;
        edges.0[l].y = y;
        edges.0[r].y = y;
    }
}

// SkScan::try_blit_fat_anti_rect / SkBlitter::blitFatAntiRect. The mask
// additive fast path bypasses edge quantization for recognized rectangles.
pub(super) fn fat_rect(path: &pm_skia_path::Path, dev: &mut Device) -> bool {
    use pm_skia_path::PathSegment;
    let mut pts = Vec::new();
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) if pts.is_empty() => pts.push(p),
            PathSegment::LineTo(p) => pts.push(p),
            PathSegment::Close => {}
            _ => return false,
        }
    }
    if pts.first() == pts.last() {
        pts.pop();
    }
    loop {
        if pts.len() < 4 {
            return false;
        }
        let redundant = (0..pts.len()).find(|&i| {
            let (a, b, c) = (
                pts[(i + pts.len() - 1) % pts.len()],
                pts[i],
                pts[(i + 1) % pts.len()],
            );
            ((a.x == b.x && b.x == c.x) && (b.y - a.y) * (c.y - b.y) >= 0.)
                || ((a.y == b.y && b.y == c.y) && (b.x - a.x) * (c.x - b.x) >= 0.)
        });
        if let Some(i) = redundant {
            pts.remove(i);
        } else {
            break;
        }
    }
    if pts.len() != 4
        || (0..4).any(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            a.x != b.x && a.y != b.y
        })
    {
        return false;
    }
    let b = path.bounds();
    let (l, t, r, bottom) = (
        b.left().floor() as i32,
        b.top().floor() as i32,
        b.right().ceil() as i32,
        b.bottom().ceil() as i32,
    );
    if r - l < 3 {
        return false;
    }
    if bottom == t {
        return true;
    }
    let (pl, pr, mut pt, pb) = (
        l as f32 + 1. - b.left(),
        b.right() - (r - 1) as f32,
        t as f32 + 1. - b.top(),
        b.bottom() - (bottom - 1) as f32,
    );
    if bottom - t == 1 {
        pt = b.height();
    }
    let alpha = |v: f32| {
        let a = (v * 255.) as u8;
        if a > 247 {
            255
        } else if a < 8 {
            0
        } else {
            a
        }
    };
    let row = |dev: &mut Device, y: i32, h: f32| {
        dev.aa(l, y, alpha(pl * h));
        for x in l + 1..r - 1 {
            dev.aa(x, y, alpha(h));
        }
        dev.aa(r - 1, y, alpha(pr * h));
    };
    row(dev, t, pt);
    for y in t + 1..bottom - 1 {
        dev.aa(l, y, alpha(pl));
        dev.blit_h(l + 1, y, r - l - 2);
        dev.aa(r - 1, y, alpha(pr));
    }
    if bottom - t > 1 {
        row(dev, bottom - 1, pb);
    }
    true
}
