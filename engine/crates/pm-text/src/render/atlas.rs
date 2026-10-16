//! Coverage pages: every coverage item of a scene gets a cell, in device
//! pixels 1:1 with the target at an integer offset, so the paint pass reads
//! coverage without resampling.

use kurbo::{Affine, Rect};

use super::scene::{ItemContent, TextScene, blur_radius};

/// Space between cells (Vello's antialiasing never reaches past one pixel
/// outside a path's bounds; cells already include that pixel).
pub const GUTTER: u32 = 1;

/// One item's cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub item: usize,
    pub page: usize,
    /// Position in the page.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// The device pixel at the cell's top-left corner.
    pub device_x: i64,
    pub device_y: i64,
    /// The item's ink extends past the cell (it reaches outside the area
    /// that can be seen), so coverage must be clipped to the cell.
    pub clip: bool,
}

impl Cell {
    /// Device space to page space for this cell's paths.
    pub fn transform(&self) -> Affine {
        Affine::translate((
            self.x as f64 - self.device_x as f64,
            self.y as f64 - self.device_y as f64,
        ))
    }

    /// The cell in page space.
    pub fn rect(&self) -> Rect {
        Rect::new(
            f64::from(self.x),
            f64::from(self.y),
            f64::from(self.x + self.width),
            f64::from(self.y + self.height),
        )
    }
}

/// The pages and cells of a scene.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Atlas {
    pub pages: Vec<(u32, u32)>,
    pub cells: Vec<Cell>,
}

/// The whole-pixel device rect an item's coverage must hold: its ink, one
/// pixel of antialiasing, limited to what can reach the visible target
/// (the target grown by the item's blur radius).
fn cell_rect(scene: &TextScene, bounds: Rect, blur: f64) -> Option<(i64, i64, i64, i64, bool)> {
    let r = bounds.inflate(1.0, 1.0);
    let reach = f64::from(blur_radius(blur));
    let visible = Rect::new(
        -reach,
        -reach,
        f64::from(scene.width) + reach,
        f64::from(scene.height) + reach,
    );
    let clipped = r.intersect(visible);
    if clipped.width() <= 0.0 || clipped.height() <= 0.0 {
        return None;
    }
    let x0 = clipped.x0.floor() as i64;
    let y0 = clipped.y0.floor() as i64;
    let x1 = clipped.x1.ceil() as i64;
    let y1 = clipped.y1.ceil() as i64;
    let clip = r.x0 < visible.x0 || r.y0 < visible.y0 || r.x1 > visible.x1 || r.y1 > visible.y1;
    Some((x0, y0, x1, y1, clip))
}

/// Packs the coverage items of `scene` into pages no larger than
/// `max_page` px a side (shelves in item order).
pub fn pack(scene: &TextScene, max_page: u32) -> Atlas {
    let mut atlas = Atlas::default();
    let mut page: Option<usize> = None;
    let (mut shelf_x, mut shelf_y, mut shelf_h) = (0u32, 0u32, 0u32);
    // Page width: the widest cell, at least 1024 px, at most `max_page`.
    let rects: Vec<_> = scene
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| matches!(it.content, ItemContent::Coverage { .. }))
        .filter_map(|(i, it)| cell_rect(scene, it.bounds, it.blur).map(|r| (i, r)))
        .collect();
    let widest = rects
        .iter()
        .map(|(_, r)| (r.2 - r.0) as u32)
        .max()
        .unwrap_or(0);
    let page_w = widest.clamp(1024.min(max_page), max_page);
    // Items larger than a page are split into page-sized tiles, each
    // clipped to its own rect.
    let mut tiles = Vec::with_capacity(rects.len());
    for (item, (x0, y0, x1, y1, clip)) in rects {
        let step = i64::from(max_page);
        let split = x1 - x0 > step || y1 - y0 > step;
        let mut ty = y0;
        while ty < y1 {
            let mut tx = x0;
            while tx < x1 {
                let (tx1, ty1) = ((tx + step).min(x1), (ty + step).min(y1));
                tiles.push((item, tx, ty, tx1, ty1, clip || split));
                tx = tx1;
            }
            ty = (ty + step).min(y1);
        }
    }
    for (item, x0, y0, x1, y1, clip) in tiles {
        let w = (x1 - x0) as u32;
        let h = (y1 - y0) as u32;
        if page.is_none() || shelf_x + w > page_w {
            shelf_y += shelf_h + if shelf_h > 0 { GUTTER } else { 0 };
            shelf_x = 0;
            shelf_h = 0;
        }
        if page.is_none() || shelf_y + h > max_page {
            atlas.pages.push((page_w, 0));
            page = Some(atlas.pages.len() - 1);
            shelf_x = 0;
            shelf_y = 0;
            shelf_h = 0;
        }
        let p = page.expect("a page exists");
        atlas.cells.push(Cell {
            item,
            page: p,
            x: shelf_x,
            y: shelf_y,
            width: w,
            height: h,
            device_x: x0,
            device_y: y0,
            clip,
        });
        shelf_x += w + GUTTER;
        shelf_h = shelf_h.max(h);
        let used = &mut atlas.pages[p].1;
        *used = (*used).max(shelf_y + h);
    }
    atlas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::scene::{fill_path, rounded_rect};

    #[test]
    fn cells_do_not_overlap_and_map_device_pixels() {
        let mut scene = TextScene {
            width: 3000,
            height: 200,
            items: vec![],
        };
        for i in 0..40 {
            let x = 10.0 + f64::from(i) * 70.0;
            scene.items.push(
                fill_path(rounded_rect(x, 20.0, 60.0, 30.0, 4.0), Affine::IDENTITY, [1.0; 4], 1.0)
                    .unwrap(),
            );
        }
        let atlas = pack(&scene, 128);
        assert_eq!(atlas.cells.len(), 40);
        assert!(atlas.pages.len() > 1);
        for (i, a) in atlas.cells.iter().enumerate() {
            let (pw, ph) = atlas.pages[a.page];
            assert!(a.x + a.width <= pw && a.y + a.height <= ph);
            for b in &atlas.cells[i + 1..] {
                if a.page == b.page {
                    let ra = a.rect().inflate(-0.5, -0.5);
                    assert!(ra.intersect(b.rect()).area() <= 0.0);
                }
            }
            // The item's device origin lands on the cell's corner.
            let p = a.transform() * kurbo::Point::new(a.device_x as f64, a.device_y as f64);
            assert_eq!((p.x, p.y), (f64::from(a.x), f64::from(a.y)));
            assert!(!a.clip);
        }
    }

    #[test]
    fn large_items_are_tiled() {
        let scene = TextScene {
            width: 9000,
            height: 300,
            items: vec![
                fill_path(rounded_rect(10.0, 10.0, 8900.0, 200.0, 0.0), Affine::IDENTITY, [1.0; 4], 1.0)
                    .unwrap(),
            ],
        };
        let atlas = pack(&scene, 4096);
        assert_eq!(atlas.cells.len(), 3);
        let covered: i64 = atlas.cells.iter().map(|c| i64::from(c.width)).sum();
        assert_eq!(covered, 8902);
        assert!(atlas.cells.iter().all(|c| c.clip && c.width <= 4096));
        assert_eq!(atlas.cells[1].device_x, atlas.cells[0].device_x + 4096);
    }

    #[test]
    fn ink_outside_the_target_is_clipped() {
        let scene = TextScene {
            width: 100,
            height: 100,
            items: vec![
                fill_path(rounded_rect(-50.0, 10.0, 100.0, 10.0, 0.0), Affine::IDENTITY, [1.0; 4], 1.0)
                    .unwrap(),
            ],
        };
        let atlas = pack(&scene, 4096);
        assert_eq!(atlas.cells[0].device_x, 0);
        assert!(atlas.cells[0].clip);
    }
}
