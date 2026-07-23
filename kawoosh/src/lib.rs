use std::env;

use gpui::{
    Action, App, Entity, FocusHandle, Focusable, Role, ScrollHandle, Window, colors::DefaultColors,
    div, prelude::*,
};
use smallvec::{SmallVec, smallvec};

pub mod actions;
mod constants;
mod content;
mod keybinds;
mod store;
mod tab;

use crate::{actions::*, constants::DEFAULT_FONT, content::Content, store::Store, tab::Tab};

pub use keybinds::default_keybinds;

#[derive(Debug)]
pub struct Kawoosh {
    pub store: Entity<Store>,
    pub focus_handle: FocusHandle,
    pub tabs_scroll: ScrollHandle,
    pub tabs: SmallVec<[Entity<Tab>; 4]>,
    pub selected_idx: usize,
}

impl Kawoosh {
    pub fn new(cx: &mut App, focus_handle: FocusHandle) -> Entity<Self> {
        let store = cx.new(|_| Store::new(env::current_dir().ok()));
        let buffer = store.update(cx, |store, cx| {
            let cwd = store.cwd.clone();
            store.new_directory(cwd, cx)
        });
        let content = Content::for_buffer(&buffer, cx);
        let tabs = smallvec![cx.new(|cx| Tab::new(
            0,
            cx.new(|_| content),
            focus_handle.clone(),
            true,
            cx
        ))];
        cx.new(|_cx| Self {
            store,
            focus_handle,
            tabs_scroll: ScrollHandle::new(),
            tabs,
            selected_idx: 0,
        })
    }

    fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_idx == index && index != 0 {
            return;
        }

        if let Some(tab) = self.tabs.get(self.selected_idx) {
            tab.update(cx, |tab, cx| {
                tab.selected = false;
                cx.notify();
            })
        }
        self.selected_idx = if index == self.tabs.len() {
            0
        } else {
            index.min(self.tabs.len().saturating_sub(1))
        };
        if let Some(tab) = self.tabs.get(self.selected_idx) {
            tab.update(cx, |tab, cx| {
                tab.selected = true;
                let handle = tab.content.read(cx).focus_handle(cx);
                handle.focus(window, cx);
                cx.notify();
            });
            self.tabs_scroll.scroll_to_top_of_item(self.selected_idx);
        }
        cx.notify();
    }

    fn update_tabs_idx(&mut self, cx: &mut Context<Self>) {
        for (i, tab) in self.tabs.iter().enumerate() {
            tab.update(cx, |tab, cx| {
                if tab.idx != i {
                    tab.idx = i;
                    cx.notify();
                }
            });
        }
    }

    fn open_in_selected_tab(
        &mut self,
        buffer: store::AnyBuffer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content = Content::for_buffer(&buffer, cx);
        if let Some(tab) = self.tabs.get(self.selected_idx).cloned() {
            tab.update(cx, |tab, cx| {
                tab.content.update(cx, |slot, cx| {
                    *slot = content;
                    cx.notify();
                });
            });
            let handle = tab.read(cx).content.read(cx).focus_handle(cx);
            handle.focus(window, cx);
        }
    }
}

impl Render for Kawoosh {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let colors = cx.default_colors();
        let active_content = self
            .tabs
            .get(self.selected_idx)
            .map(|tab| tab.read(cx).content.clone());
        div()
            .id("root")
            .track_focus(&self.focus_handle(cx))
            .role(Role::Window)
            .on_action(
                cx.listener(|this: &mut Self, action: &TabSelect, window, cx| {
                    this.select_tab(action.idx, window, cx);
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|this: &mut Self, _: &TabNext, window, cx| {
                this.select_tab(this.selected_idx.wrapping_add(1), window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this: &mut Self, _: &TabPrev, window, cx| {
                this.select_tab(this.selected_idx.wrapping_sub(1), window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this: &mut Self, _: &TabNew, window, cx| {
                let buffer = this.store.update(cx, |store, cx| {
                    let cwd = store.cwd.clone();
                    store.new_directory(cwd, cx)
                });
                let content = Content::for_buffer(&buffer, cx);
                let focus_handle = this.focus_handle.clone();
                let tab = cx.new(|cx| Tab::new(0, cx.new(|_| content), focus_handle, true, cx));
                this.tabs.push(tab);
                this.select_tab(this.tabs.len() - 1, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, e: &TabClose, window, cx| {
                if this.tabs.len() == 1 {
                    window.dispatch_action(Quit.boxed_clone(), cx);
                } else {
                    let idx = e.idx.unwrap_or(this.selected_idx);
                    let is_last = idx == this.tabs.len() - 1;
                    this.tabs.remove(idx);
                    if is_last {
                        this.update_tabs_idx(cx);
                    }
                    if idx <= this.selected_idx {
                        this.select_tab(this.selected_idx.saturating_sub(1), window, cx);
                    }
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, action: &OpenPath, window, cx| {
                let path = action.path.clone();
                let buffer = if path.is_dir() {
                    this.store
                        .update(cx, |store, cx| store.new_directory(path, cx))
                } else {
                    this.store
                        .update(cx, |store, cx| store.open_document(path, cx))
                };
                this.open_in_selected_tab(buffer, window, cx);
                cx.notify();
            }))
            .flex()
            .flex_col()
            .font_family(DEFAULT_FONT)
            .w_full()
            .h_full()
            .bg(colors.background)
            .child(
                div()
                    .id("tabs")
                    .role(Role::TabList)
                    .flex()
                    .flex_row()
                    .justify_evenly()
                    .overflow_x_scroll()
                    .track_scroll(&self.tabs_scroll)
                    .tab_group()
                    .children(self.tabs.iter().cloned()),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .p_2()
                    .children(active_content),
            )
    }
}

impl Focusable for Kawoosh {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
