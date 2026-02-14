use gpui::{AnyElement, Context, IntoElement as _, ParentElement as _, Styled as _, div, px};
use gpui_component::{ActiveTheme as _, StyledExt as _, v_flex};
use tokio::sync::watch;

use super::AgntGui;
use crate::typeahead::{
    ActiveTypeahead, TypeaheadActivation, TypeaheadItem, TypeaheadMatchSet, TypeaheadState,
    TypeaheadWindowItem, build_typeahead_window_items,
};

pub(super) struct GuiTypeahead {
    state: TypeaheadState,
}

impl GuiTypeahead {
    pub(super) fn new_for_current_project() -> Self {
        Self {
            state: TypeaheadState::new_for_current_project(),
        }
    }

    pub(super) fn updates(&self) -> [watch::Receiver<u64>; 2] {
        self.state.updates()
    }

    pub(super) fn activate_selected(
        &mut self,
        input: &str,
        cursor_pos: usize,
    ) -> Option<TypeaheadActivation> {
        self.state.activate_selected(input, cursor_pos)
    }

    pub(super) fn dismiss_if_visible(&mut self, input: &str, cursor_pos: usize) -> bool {
        if self.state.visible_matches(input, cursor_pos).is_none() {
            return false;
        }
        self.state.dismiss(input, cursor_pos);
        true
    }

    pub(super) fn move_if_visible(
        &mut self,
        direction: i32,
        input: &str,
        cursor_pos: usize,
    ) -> bool {
        if self.state.visible_matches(input, cursor_pos).is_none() {
            return false;
        }
        self.state.move_selection(direction, input, cursor_pos);
        true
    }

    pub(super) fn render_panel(
        &mut self,
        input: &str,
        cursor_pos: usize,
        cx: &Context<AgntGui>,
    ) -> Option<AnyElement> {
        let active = self.state.visible_matches(input, cursor_pos)?;
        let selected_index = self.state.selected_index();
        let window_start = self.state.window_start();
        Some(match active {
            ActiveTypeahead::Command(set) => {
                Self::render_match_set(&set, selected_index, window_start, cx)
            }
            ActiveTypeahead::Mention(set) => {
                Self::render_match_set(&set, selected_index, window_start, cx)
            }
        })
    }

    fn render_match_set<T: TypeaheadItem>(
        set: &TypeaheadMatchSet<T>,
        selected_index: usize,
        window_start: usize,
        cx: &Context<AgntGui>,
    ) -> AnyElement {
        let header = if set.query.is_empty() {
            "Suggestions".to_string()
        } else {
            format!("Suggestions for `{}`", set.query)
        };

        let mut list = v_flex()
            .w_full()
            .gap_1()
            .p_2()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .bg(cx.theme().muted)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child(header),
            );

        if set.matches.is_empty() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(typeahead_status_text(set)),
            );
        } else {
            let max_items = 4usize;
            let rows = build_typeahead_window_items(set.matches.len(), window_start, max_items);
            for row in rows {
                list = match row {
                    TypeaheadWindowItem::Value(absolute_index) => list.child(
                        Self::render_match_row(set, absolute_index, selected_index, cx),
                    ),
                    TypeaheadWindowItem::Divider => list.child(Self::render_match_divider(cx)),
                };
            }
        }

        list.into_any_element()
    }

    fn render_match_row<T: TypeaheadItem>(
        set: &TypeaheadMatchSet<T>,
        absolute_index: usize,
        selected_index: usize,
        cx: &Context<AgntGui>,
    ) -> AnyElement {
        let item = &set.matches[absolute_index];
        let marker = if absolute_index == selected_index {
            "› "
        } else {
            "  "
        };
        let mut line = format!("{marker}{}", item.token_text());
        if let Some(description) = item.description() {
            line.push_str("  ");
            line.push_str(&description);
        }

        let mut row = div()
            .w_full()
            .h_5()
            .px_1()
            .flex()
            .items_center()
            .text_sm()
            .child(line);
        if absolute_index == selected_index {
            row = row.text_color(cx.theme().cyan);
        } else {
            row = row.text_color(cx.theme().foreground);
        }
        row.into_any_element()
    }

    fn render_match_divider(cx: &Context<AgntGui>) -> AnyElement {
        div()
            .w_full()
            .h_5()
            .px_1()
            .flex()
            .items_center()
            .child(div().w_full().h(px(1.)).bg(cx.theme().border))
            .into_any_element()
    }
}

fn typeahead_status_text<T: TypeaheadItem>(set: &TypeaheadMatchSet<T>) -> &'static str {
    if set.loading && set.show_loading {
        match set.leader {
            '@' => {
                if set.query.is_empty() {
                    "indexing files..."
                } else {
                    "searching..."
                }
            }
            _ => "loading...",
        }
    } else if set.loading {
        " "
    } else {
        "no matches"
    }
}
