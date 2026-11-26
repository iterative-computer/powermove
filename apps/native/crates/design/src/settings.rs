//! Settings, the sign-in sheet and the visual pickers: the macOS settings
//! scale (13 pt rows, 11 pt secondary text, 28 pt sidebar rows with small
//! tiles, regular controls) and the drawn art those surfaces carry (section
//! tiles, picker wallpapers). Nothing in those views names a size or a
//! colour of its own: it reads this group, `TextSize`, `Ink` and `Colors`.
use crate::Scheme;
use serde::{Deserialize, Serialize};

/// A drawn tile's material: a squircle lit from the top, with an inner edge
/// and a glyph pressed into it (colours 0xRRGGBB, opacities 0–1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TilePalette {
    /// The body's gradient, top to bottom.
    pub body_top: u32,
    pub body_bottom: u32,
    /// The inner edge's colour and its opacity at the top and the bottom.
    pub edge: u32,
    pub edge_top: f32,
    pub edge_bottom: f32,
    /// The soft top light (white) and its opacity.
    pub light: u32,
    pub light_opacity: f32,
    /// The glyph's gradient, top to bottom.
    pub glyph_top: u32,
    pub glyph_bottom: u32,
    /// The tile's drop shadow: colour, offset, blur, opacity.
    pub shadow: u32,
    pub shadow_offset: f32,
    pub shadow_blur: f32,
    pub shadow_opacity: f32,
    /// The glyph's own offset shadow (pressed in): colour and opacity.
    pub press: u32,
    pub press_opacity: f32,
    /// The signed-out account glyph.
    pub account_glyph: u32,
}

/// The visual pickers' art (Appearance, Open Projects In): a rounded tile on
/// a wallpaper in Powermove's dusk, a crisp mini window inside it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PickerPalette {
    /// The wallpaper's sky, top to bottom, and the two soft folds over it.
    pub sky_top: u32,
    pub sky_mid: u32,
    pub sky_bottom: u32,
    pub fold: u32,
    pub fold_opacity: f32,
    pub glow: u32,
    pub glow_opacity: f32,
    /// A light sheen along the fold (silk catching the light).
    pub sheen: u32,
    pub sheen_opacity: f32,
    /// The mini window's traffic lights.
    pub close: u32,
    pub minimize: u32,
    pub zoom: u32,
    /// The mini window's shadow opacity and its edge (ink) opacity.
    pub window_shadow: f32,
    pub window_edge: f32,
    /// An unselected tile's edge (ink) opacity.
    pub tile_edge: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The window: its width (fixed), its least and first heights, the
    /// sidebar's width.
    pub window_width: f32,
    pub window_height: f32,
    pub window_default_height: f32,
    pub sidebar: f32,
    /// The sidebar's rows: the highlight's height, the tile in it and the
    /// tile's corner; the highlight's side inset from the sidebar's edges.
    /// Rows stand `metrics.list_gap` apart. The tile sits
    /// `(sidebar_row − sidebar_tile) / 2` inside its highlight on every side,
    /// and the highlight's corner is concentric (`tile_radius` + that inset).
    pub sidebar_row: f32,
    pub sidebar_tile: f32,
    pub tile_radius: f32,
    pub sidebar_inset: f32,
    /// The account row and its avatar.
    pub account_row: f32,
    pub avatar: f32,
    /// The page header on the traffic lights' line: its band and tile.
    pub header_height: f32,
    pub header_tile: f32,
    /// The form: the page's side inset, top and bottom room, the column's
    /// greatest width, the space between groups, a row's side inset and
    /// vertical padding, a row's least height, a group card's corner, the
    /// air under a row's second line.
    pub page_inset: f32,
    pub page_top: f32,
    pub page_bottom: f32,
    pub column: f32,
    pub section_gap: f32,
    pub row_inset: f32,
    pub row_padding: f32,
    pub row: f32,
    pub group_radius: f32,
    pub title_air: f32,
    /// The ⓘ help popover opens after the pointer rests this long (s).
    pub help_delay: f32,
    /// Control widths in a row: a secure field, a progress bar, a folder's
    /// path pop-up (at most), a filter field (at least) and a value's least.
    pub secure_width: f32,
    pub progress_width: f32,
    pub path_width: f32,
    pub filter_width: f32,
    pub value_least: f32,
    /// The ⓘ popover's text width.
    pub help_width: f32,
    /// A visual picker's picture, the gap to its ring, its corner, the
    /// ring's width and the mini window's corner inside it.
    pub picker_width: f32,
    pub picker_height: f32,
    pub picker_ring_gap: f32,
    pub picker_ring: f32,
    pub picker_radius: f32,
    pub picker_window_radius: f32,
    /// A picker option's column (its tile and its label under it): every
    /// picker is one left-aligned grid of these, `space.lg` apart.
    pub picker_column: f32,
    /// Settings › Account's credits: the meter's thickness, the daily
    /// chart's height, the room between its bars and the air above each.
    pub credits_meter: f32,
    pub credits_chart: f32,
    pub credits_bar_gap: f32,
    pub credits_air: f32,
    /// Settings › Agents' app marks and the space after them.
    pub app_icon: f32,
    pub app_icon_gap: f32,
    /// The dark sidebar's veil over the system material (Raycast's depth),
    /// and a hovered row's wash relative to the selection.
    pub dark_veil: f32,
    pub hover_opacity: f32,
    /// Weights (CSS numbers): the page header, a sidebar group's name, the
    /// account's name and initial.
    pub header_weight: f32,
    /// A group's heading and a picker's chosen label.
    pub heading_weight: f32,
    pub group_weight: f32,
    pub account_weight: f32,
    pub tile: TilePalette,
    pub picker: PickerPalette,
}

impl Settings {
    pub fn of(scheme: Scheme) -> Self {
        let dark = scheme == Scheme::Dark;
        Self {
            window_width: 700.,
            window_height: 500.,
            window_default_height: 600.,
            sidebar: 210.,
            sidebar_row: 30.,
            sidebar_tile: 20.,
            tile_radius: 5.,
            sidebar_inset: 10.,
            account_row: 40.,
            avatar: 26.,
            header_height: 30.,
            header_tile: 20.,
            page_inset: 20.,
            page_top: 4.,
            page_bottom: 20.,
            column: 560.,
            section_gap: 14.,
            row_inset: 12.,
            row_padding: 6.,
            row: 38.,
            group_radius: 10.,
            title_air: 4.,
            help_delay: 0.3,
            help_width: 260.,
            secure_width: 160.,
            progress_width: 96.,
            path_width: 210.,
            filter_width: 175.,
            value_least: 88.,
            picker_width: 96.,
            picker_height: 64.,
            picker_ring_gap: 3.,
            picker_ring: 2.,
            picker_radius: 9.,
            picker_window_radius: 4.,
            picker_column: 120.,
            credits_meter: 6.,
            credits_chart: 44.,
            credits_bar_gap: 2.,
            credits_air: 8.,
            app_icon: 20.,
            app_icon_gap: 10.,
            dark_veil: 0.37,
            hover_opacity: 0.5,
            header_weight: 700.,
            heading_weight: 600.,
            group_weight: 600.,
            account_weight: 600.,
            tile: if dark {
                TilePalette {
                    body_top: 0x55555c,
                    body_bottom: 0x26262b,
                    edge: 0xffffff,
                    edge_top: 0.42,
                    edge_bottom: 0.06,
                    light: 0xffffff,
                    light_opacity: 0.24,
                    glyph_top: 0xffffff,
                    glyph_bottom: 0xd4d4da,
                    shadow: 0x000000,
                    shadow_offset: 0.6,
                    shadow_blur: 0.7,
                    shadow_opacity: 0.6,
                    press: 0x000000,
                    press_opacity: 0.5,
                    account_glyph: 0x98989f,
                }
            } else {
                TilePalette {
                    body_top: 0xffffff,
                    body_bottom: 0xe2e2e6,
                    edge: 0x000000,
                    edge_top: 0.08,
                    edge_bottom: 0.2,
                    light: 0xffffff,
                    light_opacity: 0.9,
                    glyph_top: 0x55555b,
                    glyph_bottom: 0x232327,
                    shadow: 0x000000,
                    shadow_offset: 0.6,
                    shadow_blur: 0.7,
                    shadow_opacity: 0.2,
                    press: 0xffffff,
                    press_opacity: 0.9,
                    account_glyph: 0x6a6a70,
                }
            },
            // Powermove's dusk: a deep indigo sky warming to the accent at
            // the horizon, a darker fold across it and a warm glow low down.
            picker: if dark {
                PickerPalette {
                    sky_top: 0x1e1b4b,
                    sky_mid: 0x5b2a86,
                    sky_bottom: 0xf0703a,
                    fold: 0x0d0b2a,
                    fold_opacity: 0.5,
                    glow: 0xff8a3d,
                    glow_opacity: 0.5,
                    sheen: 0xffffff,
                    sheen_opacity: 0.14,
                    close: 0xff5f57,
                    minimize: 0xfebc2e,
                    zoom: 0x28c840,
                    window_shadow: 0.45,
                    window_edge: 0.16,
                    tile_edge: 0.12,
                }
            } else {
                PickerPalette {
                    sky_top: 0x4c4fc4,
                    sky_mid: 0x9b5cc4,
                    sky_bottom: 0xff9a5a,
                    fold: 0x2e2f8a,
                    fold_opacity: 0.35,
                    glow: 0xffb36b,
                    glow_opacity: 0.5,
                    sheen: 0xffffff,
                    sheen_opacity: 0.22,
                    close: 0xff5f57,
                    minimize: 0xfebc2e,
                    zoom: 0x28c840,
                    window_shadow: 0.3,
                    window_edge: 0.12,
                    tile_edge: 0.1,
                }
            },
        }
    }
    /// The tile's inset inside its row highlight, on every side.
    pub fn tile_inset(&self) -> f32 {
        (self.sidebar_row - self.sidebar_tile) / 2.
    }
    /// The highlight's corner: concentric with the tile's.
    pub fn selection_radius(&self) -> f32 {
        self.tile_radius + self.tile_inset()
    }
}

/// The welcome window (the first run's stage) and the sign-in sheet that
/// shares its views: every place and size they draw at.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    /// Each half of the stage's window, and the column's width.
    pub half: f32,
    pub column: f32,
    /// The window picture on the field: its scale and its corner on it.
    pub window_scale: f32,
    pub window_left: f32,
    pub window_top: f32,
    /// The stage's grouped list: corner, a row's least height, the scroll's
    /// greatest height, its edge fade, how much wider than the column it
    /// runs, a row's side inset, the icons' tile.
    pub list_radius: f32,
    pub list_row: f32,
    pub list_max_height: f32,
    pub list_fade: f32,
    pub list_widen: f32,
    pub list_inset: f32,
    pub icon_tile: f32,
    /// The stage's primary action, the mark's height from the top, where the
    /// actions start and the room left above them.
    pub primary_width: f32,
    pub primary_height: f32,
    pub logo_top: f32,
    pub actions_top: f32,
    pub actions_room: f32,
    /// The sign-in sheet: its width and first height, the column's inset
    /// from its edges, its controls' height, its tile, Cancel's place.
    pub sheet_width: f32,
    pub sheet_height: f32,
    pub sheet_padding: f32,
    pub sheet_control: f32,
    pub sheet_tile: f32,
    pub sheet_foot: f32,
    pub sheet_foot_gap: f32,
    /// The six code cells, and the email's well (one corner for both).
    pub cell_width: f32,
    pub cell_height: f32,
    pub field_height: f32,
    pub field_radius: f32,
}

impl Welcome {
    pub const fn new() -> Self {
        Self {
            half: 450.,
            column: 330.,
            window_scale: 0.88,
            window_left: 32.,
            window_top: 104.,
            list_radius: 12.,
            list_row: 50.,
            list_max_height: 224.,
            list_fade: 18.,
            list_widen: 20.,
            list_inset: 16.,
            icon_tile: 32.,
            primary_width: 280.,
            primary_height: 36.,
            logo_top: 104.,
            actions_top: 510.,
            actions_room: 20.,
            sheet_width: 400.,
            sheet_height: 428.,
            sheet_padding: 32.,
            sheet_control: 36.,
            sheet_tile: 48.,
            sheet_foot: 22.,
            sheet_foot_gap: 20.,
            cell_width: 38.,
            cell_height: 48.,
            field_height: 40.,
            field_radius: 10.,
        }
    }
    /// The sheet's column: its width less its padding on both sides.
    pub fn sheet_inner(&self) -> f32 {
        self.sheet_width - 2. * self.sheet_padding
    }
}

/// Text sizes as steps (pt), like a utility scale: views pick a step, never
/// a number. `base` is the macOS body (13 pt).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextSize {
    pub xs: f32,
    pub sm: f32,
    pub base: f32,
    pub lg: f32,
    pub xl: f32,
    pub x2l: f32,
    pub x3l: f32,
    pub x4l: f32,
}

impl TextSize {
    pub const fn new() -> Self {
        Self {
            xs: 11.,
            sm: 12.,
            base: 13.,
            lg: 15.,
            xl: 17.,
            x2l: 20.,
            x3l: 22.,
            x4l: 26.,
        }
    }
}

/// Ink alpha steps (0–1) for `text` (or `accent`) drawn over a surface:
/// washes, wells, edges. Views pick a step, never an alpha.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ink {
    /// A hovered row's faint wash.
    pub wash: f32,
    /// A field's or a cell's quiet fill; a surface a step above its ground.
    pub well: f32,
    /// A well's or a hairline's edge.
    pub edge: f32,
    /// An accent tint behind a mark.
    pub tint: f32,
}

impl Ink {
    pub const fn new() -> Self {
        Self {
            wash: 0.03,
            well: 0.05,
            edge: 0.12,
            tint: 0.15,
        }
    }
}

/// `rgb` (0xRRGGBB) with `alpha` (0–1) as 0xRRGGBBAA, for GPUI's `rgba`.
pub fn with_alpha(rgb: u32, alpha: f32) -> u32 {
    (rgb << 8) | (alpha.clamp(0., 1.) * 255.).round() as u32
}

/// `rgb` as an SVG colour, `#rrggbb`.
pub fn css(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xff_ffff)
}
