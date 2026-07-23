use gpui::{
    Action, App, Context, Entity, FocusHandle, Focusable, IntoElement, MouseButton, Render, Role,
    Window, colors::DefaultColors, div, prelude::*, rems,
};

use crate::content::Content;
use crate::{TabClose, TabSelect};

#[derive(Debug)]
pub struct Tab {
    pub idx: usize,
    pub content: Entity<Content>,
    pub selected: bool,
    focus_handle: FocusHandle,
}

impl Tab {
    pub fn new(
        idx: usize,
        content: Entity<Content>,
        focus_handle: FocusHandle,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&content, |_this, _content, cx| cx.notify())
            .detach();
        Self {
            idx,
            content,
            selected,
            focus_handle,
        }
    }
}

impl Render for Tab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors();
        let text_size = rems(1.);
        let (bg, text_color) = match self.selected {
            false => (colors.container, colors.text),
            true => (colors.selected, colors.selected_text),
        };
        let idx = self.idx;
        let title = self.content.read(cx).title(cx);
        div()
            .id(("tab", self.idx))
            .track_focus(&self.focus_handle)
            .tab_index(self.idx as isize)
            .role(Role::Tab)
            .flex_1()
            .flex()
            .items_center()
            .bg(bg)
            .text_color(text_color)
            .px_1()
            .line_height(text_size)
            .text_size(text_size)
            .h(text_size + rems(0.25))
            .cursor_pointer()
            .when(!self.selected, |this| {
                this.hover(|s| {
                    s.bg(colors.selected)
                        .text_color(colors.selected_text)
                        .opacity(0.8)
                })
            })
            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                window.dispatch_action(TabSelect { idx }.boxed_clone(), cx);
            })
            .on_mouse_down(MouseButton::Middle, move |_event, window, cx| {
                window.dispatch_action(TabClose { idx: Some(idx) }.boxed_clone(), cx);
            })
            .on_mouse_down(MouseButton::Right, move |_event, window, cx| {
                window.dispatch_action(TabClose { idx: Some(idx) }.boxed_clone(), cx);
            })
            .child(title)
    }
}

impl Focusable for Tab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
