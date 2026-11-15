//! Selection on the existing shaped prose; Markdown styling stays in StyledText.
use gpui::*;
use pm_surface::Id;
use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};
#[derive(Clone)]
pub struct Block {
    pub owner: Id,
    pub offset: usize,
    pub text: String,
    pub layout: TextLayout,
}
#[derive(Default)]
pub struct Selection {
    pub blocks: HashMap<Id, Block>,
    pub owner: Option<Id>,
    pub anchor: usize,
    pub head: usize,
    pub dragging: bool,
    pub revealed: Option<Id>,
    pub reveal_seen: Option<(Id, u64)>,
}
impl Selection {
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
    pub fn hit(&self, owner: Id, point: Point<Pixels>) -> Option<usize> {
        let mut blocks: Vec<_> = self.blocks.values().filter(|b| b.owner == owner).collect();
        blocks.sort_by_key(|b| b.offset);
        let block = blocks
            .iter()
            .find(|b| b.layout.bounds().contains(&point))
            .copied()
            .or_else(|| {
                blocks
                    .iter()
                    .min_by(|a, b| {
                        let distance =
                            |b: &&Block| (f32::from(b.layout.bounds().center().y - point.y)).abs();
                        distance(a).total_cmp(&distance(b))
                    })
                    .copied()
            })?;
        let at = block
            .layout
            .index_for_position(point)
            .unwrap_or_else(|i| i)
            .min(block.text.len());
        Some(block.offset + at)
    }
    pub fn text(&self, owner: Id) -> String {
        let mut blocks: Vec<_> = self.blocks.values().filter(|b| b.owner == owner).collect();
        blocks.sort_by_key(|b| b.offset);
        blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn end_position(&self) -> Option<Point<Pixels>> {
        let owner = self.owner?;
        let at = self.range().end;
        let b = self
            .blocks
            .values()
            .find(|b| b.owner == owner && at >= b.offset && at <= b.offset + b.text.len())?;
        b.layout.position_for_index(at - b.offset)
    }
}
pub struct Selectable {
    pub text: StyledText,
    pub block: Block,
    pub id: Id,
    pub selection: Rc<RefCell<Selection>>,
    pub color: Hsla,
}
impl IntoElement for Selectable {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Selectable {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.text.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text.prepaint(id, inspector, bounds, state, window, cx);
        self.block.layout = self.text.layout().clone();
        let mut selected = self.selection.borrow_mut();
        let new = selected
            .blocks
            .insert(self.id, self.block.clone())
            .is_none();
        if new && selected.revealed == Some(self.block.owner) {
            window.refresh();
        }
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let selected = self.selection.borrow();
        if selected.owner == Some(self.block.owner) {
            let range = selected.range();
            let local = range.start.saturating_sub(self.block.offset)
                ..range
                    .end
                    .saturating_sub(self.block.offset)
                    .min(self.block.text.len());
            for (i, c) in self
                .block
                .text
                .char_indices()
                .filter(|(i, _)| local.contains(i))
            {
                let Some(start) = self.block.layout.position_for_index(i) else {
                    continue;
                };
                let next = self.block.layout.position_for_index(i + c.len_utf8());
                let right = next
                    .filter(|p| p.y == start.y)
                    .map_or(bounds.right(), |p| p.x);
                window.paint_quad(fill(
                    Bounds::new(
                        point(start.x.min(right), start.y),
                        size(
                            (right - start.x).abs().max(px(1.)),
                            self.block.layout.line_height(),
                        ),
                    ),
                    self.color,
                ));
            }
        }
        drop(selected);
        self.text
            .paint(id, inspector, bounds, state, prepaint, window, cx);
    }
}

pub fn word_range(text: &str, at: usize) -> Range<usize> {
    let at = at.min(text.len());
    let Some(c) = text[at..]
        .chars()
        .next()
        .or_else(|| text[..at].chars().next_back())
    else {
        return 0..0;
    };
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '\'';
    let same = |other: char| {
        if word(c) {
            word(other)
        } else if c.is_whitespace() {
            other.is_whitespace()
        } else {
            false
        }
    };
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| same(*c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .take_while(|(_, c)| same(*c))
        .last()
        .map_or((at + c.len_utf8()).min(text.len()), |(i, c)| {
            at + i + c.len_utf8()
        });
    start..end
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn words_keep_unicode_and_exclude_punctuation() {
        assert_eq!(&"Hello, 日本語!"[word_range("Hello, 日本語!", 1)], "Hello");
        assert_eq!(
            &"Hello, 日本語!"[word_range("Hello, 日本語!", 10)],
            "日本語"
        );
    }
    #[test]
    fn ranges_stay_inside_one_message_across_blocks_and_reverse_drags() {
        let mut selection = Selection::default();
        for (id, owner, offset, text) in [
            (1, 10, 0, "First 😀"),
            (2, 10, 11, "Second"),
            (3, 20, 0, "Other message"),
        ] {
            selection.blocks.insert(
                id,
                Block {
                    owner,
                    offset,
                    text: text.into(),
                    layout: TextLayout::default(),
                },
            );
        }
        selection.owner = Some(10);
        selection.anchor = 17;
        selection.head = 6;
        let text = selection.text(10);
        assert_eq!(&text[selection.range()], "😀\nSecond");
        assert!(!text.contains("Other"));
    }
}

#[cfg(test)]
mod shaped_tests {
    use super::*;
    use std::sync::Arc;
    struct Prose {
        selection: Rc<RefCell<Selection>>,
    }
    impl Render for Prose {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(110.))
                .flex()
                .flex_col()
                .text_size(px(14.))
                .children(
                    [
                        (1, 0, "First 😀 paragraph wraps here"),
                        (2, 32, "Second paragraph"),
                        (
                            3,
                            49,
                            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
                        ),
                    ]
                    .into_iter()
                    .map(|(id, offset, content)| {
                        let text = StyledText::new(content).with_highlights([(
                            0..5,
                            HighlightStyle {
                                font_weight: Some(FontWeight::BOLD),
                                ..Default::default()
                            },
                        )]);
                        Selectable {
                            block: Block {
                                owner: 10,
                                offset,
                                text: content.into(),
                                layout: text.layout().clone(),
                            },
                            text,
                            id,
                            selection: self.selection.clone(),
                            color: rgb(0x6699ff).into(),
                        }
                    }),
                )
        }
    }
    #[test]
    fn shaped_prose_hit_testing_and_positions_cover_wrapping_and_two_blocks() {
        objc2::rc::autoreleasepool(|_| {
            let selection = Rc::new(RefCell::new(Selection::default()));
            let mut cx = HeadlessAppContext::with_platform(
                Arc::new(gpui_parley::ParleyTextSystem::new(
                    gpui_parley::SystemFonts::Load,
                )),
                Arc::new(()),
                || {
                    Some(Box::new(
                        gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                    ))
                },
            );
            let state = selection.clone();
            let window = cx
                .open_window(size(px(180.), px(240.)), |_, cx| {
                    cx.new(|_| Prose { selection: state })
                })
                .unwrap();
            cx.run_until_parked();
            let _image = cx.capture_screenshot(window.into()).unwrap();
            let selected = selection.borrow();
            let first = &selected.blocks[&1];
            let second = &selected.blocks[&2];
            let point = first.layout.position_for_index(6).unwrap();
            assert_eq!(selected.hit(10, point), Some(6));
            let point = second.layout.position_for_index(3).unwrap();
            assert_eq!(selected.hit(10, point), Some(35));
            assert!(first.layout.bounds().size.height > first.layout.line_height());
            let unbroken = &selected.blocks[&3];
            assert!(unbroken.layout.bounds().size.height > unbroken.layout.line_height());
            assert!(unbroken.layout.bounds().size.width <= px(110.));
        });
    }
}

pub const CITE_COMMAND: &str = "agent.citeSelection";
pub fn cite_binding() -> pm_shell::keyboard::Binding {
    pm_shell::keyboard::Binding {
        id: "native.agent.cite.selection".into(),
        chord: "cmd+shift+.".into(),
        command: CITE_COMMAND.into(),
        args: vec![],
        priority: 300,
        in_fields: false,
        repeat: false,
        loose: false,
    }
}
pub fn cite_context(agent_prose: bool, focused: bool, range: Range<usize>) -> bool {
    agent_prose && focused && !range.is_empty()
}
#[cfg(test)]
mod resolver_tests {
    use super::*;
    use pm_shell::keyboard::{Binding, Effect, Key, Resolver};
    #[test]
    fn selected_agent_prose_cites_before_pause_and_other_contexts_fall_through() {
        for (agent, focused, range, handled) in [
            (true, true, 1..4, true),
            (true, true, 0..0, false),
            (true, false, 1..4, false),
            (false, true, 1..4, false),
        ] {
            let mut resolver = Resolver::with(vec![
                cite_binding(),
                Binding {
                    id: "test.pause".into(),
                    chord: "cmd+shift+.".into(),
                    command: "test.pauseAgent".into(),
                    args: vec![],
                    priority: 400,
                    in_fields: false,
                    repeat: false,
                    loose: false,
                },
            ]);
            let effects = resolver.submit(Key {
                token: 1,
                chord: "cmd+shift+.".into(),
                field: false,
                repeat: false,
                up: false,
                compat: false,
            });
            assert!(
                matches!(&effects[..], [Effect::Dispatch { command,.. }] if command == CITE_COMMAND)
            );
            assert_eq!(cite_context(agent, focused, range), handled);
            let effects = resolver.resolve(1, handled);
            if handled {
                assert_eq!(effects, vec![Effect::Consumed(1)]);
            } else {
                assert!(
                    matches!(&effects[..],[Effect::Dispatch { command,.. }] if command == "test.pauseAgent")
                );
            }
        }
    }
}

/// Place the pill within both its source and the visible transcript viewport.
pub fn pill_origin(
    caret: Point<Pixels>,
    source: Bounds<Pixels>,
    viewport: Bounds<Pixels>,
    pill: Size<Pixels>,
) -> Point<Pixels> {
    let left = source.left().max(viewport.left());
    let top = source.top().max(viewport.top());
    let right = (source.right().min(viewport.right()) - pill.width).max(left);
    let bottom = (source.bottom().min(viewport.bottom()) - pill.height).max(top);
    point(
        caret.x.max(left).min(right) - source.left(),
        caret.y.max(top).min(bottom) - source.top(),
    )
}
#[cfg(test)]
mod pill_tests {
    use super::*;
    #[test]
    fn a_selection_near_the_viewport_edge_keeps_the_whole_pill_visible() {
        let source = Bounds::new(point(px(20.), px(-100.)), size(px(300.), px(600.)));
        let viewport = Bounds::new(point(px(0.), px(0.)), size(px(360.), px(300.)));
        let pill = size(px(48.), px(24.));
        let local = pill_origin(point(px(310.), px(290.)), source, viewport, pill);
        let screen = source.origin + local;
        assert_eq!(screen, point(px(272.), px(276.)));
        assert!(screen.x >= viewport.left() && screen.y >= viewport.top());
        assert!(
            screen.x + pill.width <= viewport.right()
                && screen.y + pill.height <= viewport.bottom()
        );
    }
}
