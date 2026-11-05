//! Worker presentation adapter. Native hosts provide font shaping and capsule
//! measurement; Linux inspection uses an explicitly synthetic metric double.
use crate::bridge::Projection;
use pm_design::Tokens;
use pm_panel_media::{
    Availability, Download, ItemId, RowData,
    render::{Badge, CardRecipe, Error, TextMetrics, middle_truncate},
};
use serde::Serialize;
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    pub width: f64,
    pub columns: usize,
    pub row_height: f64,
}
pub trait Metrics: TextMetrics {
    fn capsule_size(&self, text: &str) -> (f64, f64);
}
#[derive(Clone, Debug, Serialize)]
pub struct Card {
    pub id: ItemId,
    pub name: String,
    pub tooltip: String,
    pub recipe: CardRecipe,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub kind_symbol: Option<&'static str>,
    pub duration: Option<String>,
    pub status: Option<&'static str>,
    pub poster_opacity: f64,
    pub progress: Option<f64>,
    pub open_composition: bool,
    pub actions_disabled: bool,
    pub actions: Vec<CardAction>,
}
#[derive(Clone, Debug, Serialize)]
pub struct CardAction {
    pub label: &'static str,
    pub symbol: &'static str,
    pub input: pm_panel_media::Input,
    pub disabled: bool,
    pub glyph_color: u32,
}
fn actions(
    row: &pm_panel_media::Row,
    compositions: usize,
    disabled: bool,
    t: &Tokens,
) -> Vec<CardAction> {
    use pm_panel_media::{Input, MenuCommand};
    let mut result = vec![];
    let mut add = |label, symbol, input, unavailable| {
        result.push(CardAction {
            label,
            symbol,
            input,
            disabled: disabled || unavailable,
            glyph_color: if disabled || unavailable {
                t.colors.muted
            } else {
                t.colors.selected_text
            },
        })
    };
    match &row.data {
        RowData::Folder(f) => add(
            "Delete Folder",
            "trash",
            Input::Invoke(MenuCommand::DeleteFolder(f.id.clone())),
            false,
        ),
        RowData::Composition(c) => {
            add(
                "Add to Timeline",
                "plus",
                Input::Invoke(MenuCommand::NestComposition(c.id.clone())),
                !c.can_nest,
            );
            add(
                "Delete Composition…",
                "trash",
                Input::Invoke(MenuCommand::DeleteComposition(c.id.clone())),
                compositions <= 1,
            );
        }
        RowData::Asset(a, r) => {
            match &r.availability {
                Availability::Ready => {
                    if a.source_path.is_some() {
                        add(
                            "Show in Finder",
                            "folder",
                            Input::Invoke(MenuCommand::Reveal(a.id.clone())),
                            false,
                        );
                    }
                    add(
                        "Add to Timeline",
                        "plus",
                        Input::Invoke(MenuCommand::AddAsset(a.id.clone())),
                        false,
                    );
                }
                Availability::Offline | Availability::Changed { .. } | Availability::Failed(_) => {
                    add(
                        "Locate File…",
                        "link",
                        Input::Invoke(MenuCommand::Replace(a.id.clone())),
                        false,
                    )
                }
                Availability::Cloud { download, .. } => add(
                    if matches!(download, Download::Failed(_)) {
                        "Retry Download"
                    } else {
                        "Download from Cloud"
                    },
                    "icloud.and.arrow.down",
                    Input::Invoke(MenuCommand::Download(a.id.clone())),
                    matches!(download, Download::Downloading),
                ),
                _ => {}
            }
            add(
                "Delete Media",
                "trash",
                Input::DeleteAsset(a.id.clone()),
                false,
            );
        }
    }
    result
}
pub trait Painter {
    fn paint(&mut self, cards: &[Card]);
}
fn duration(seconds: f64) -> String {
    let n = seconds.max(0.).round() as u64;
    format!("{}:{:02}", n / 60, n % 60)
}
/// No full-project scan or resource decode. Only the model's bounded page is
/// touched. Poster leases are resolved by the host after this plan is accepted.
pub fn prepare(
    view: &Projection,
    grid: Grid,
    t: &Tokens,
    metrics: &dyn Metrics,
    disabled: bool,
    unknown_loading: &std::collections::HashSet<String>,
    hovered: &std::collections::HashSet<ItemId>,
) -> Result<Vec<Card>, Error> {
    if grid.columns == 0
        || grid.columns > 4096
        || !grid.width.is_finite()
        || !grid.row_height.is_finite()
        || grid.row_height <= 0.
    {
        return Err(Error::InvalidGeometry);
    }
    let width =
        (grid.width - t.metrics.inset as f64 * (grid.columns + 1) as f64) / grid.columns as f64;
    view.page
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let index = view.page.offset + i;
            let x = t.metrics.inset as f64
                + (index % grid.columns) as f64 * (width + t.metrics.inset as f64);
            let y = t.metrics.gap as f64 + (index / grid.columns) as f64 * grid.row_height;
            let (name, poster, background, symbol, seconds, status, loading, progress, open) =
                match &row.data {
                    RowData::Folder(f) => (
                        f.name.clone(),
                        None,
                        None,
                        Some("folder.fill"),
                        None,
                        None,
                        false,
                        None,
                        false,
                    ),
                    RowData::Composition(c) => (
                        c.name.clone(),
                        c.poster.clone(),
                        c.background.clone(),
                        Some("rectangle.stack"),
                        c.duration,
                        None,
                        false,
                        None,
                        c.open,
                    ),
                    RowData::Asset(a, r) => {
                        let status = match &r.availability {
                            Availability::Unknown if unknown_loading.contains(&a.id) => {
                                Some("Loading…")
                            }
                            Availability::Loading => Some("Loading…"),
                            Availability::Offline => Some("Media offline"),
                            Availability::Failed(_) => Some("Media unavailable"),
                            Availability::Cloud {
                                download: Download::Downloading,
                                ..
                            } => Some("Downloading…"),
                            Availability::Cloud { .. } => Some("In cloud"),
                            _ => None,
                        };
                        let progress = r
                            .progress
                            .as_ref()
                            .filter(|p| p.total > 0 && p.completed <= p.total)
                            .map(|p| p.completed as f64 / p.total as f64);
                        let symbol = match a.kind.as_str() {
                            "video" => Some("film"),
                            "image" => Some("photo"),
                            "audio" => Some("waveform"),
                            "model" => Some("cube"),
                            _ => None,
                        };
                        (
                            a.name.clone(),
                            r.poster.clone(),
                            None,
                            symbol,
                            if a.kind == "image" { None } else { a.duration },
                            status,
                            status == Some("Loading…"),
                            progress,
                            false,
                        )
                    }
                };
            let duration = seconds.map(duration);
            let mut badges = vec![];
            let mut heights = vec![];
            for (kind, label) in [
                (Badge::Duration, duration.as_deref()),
                (Badge::Status, status),
            ] {
                if let Some(label) = label {
                    let (w, h) = metrics.capsule_size(label);
                    badges.push((kind, w));
                    heights.push(h);
                }
            }
            let actions = if (row.focused || hovered.contains(&row.id)) && view.drag.is_none() {
                actions(row, view.composition_count, disabled, t)
            } else {
                vec![]
            };
            let mut recipe = CardRecipe::new(
                (x, y),
                width,
                row.selected,
                row.focused,
                &badges,
                actions.len(),
                t,
            )?;
            recipe.measure_badges(&heights)?;
            Ok(Card {
                id: row.id.clone(),
                name: middle_truncate(&name, width, metrics),
                tooltip: name,
                recipe,
                poster,
                background,
                kind_symbol: symbol,
                duration,
                status,
                poster_opacity: if loading { 0.5 } else { 1. },
                progress,
                open_composition: open,
                actions_disabled: disabled,
                actions,
            })
        })
        .collect()
}
