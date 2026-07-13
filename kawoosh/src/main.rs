use std::sync::Arc;

use anyhow::Result;
use env_logger::Env;
use gpui::{
    Action, App, Entity, FocusHandle, Focusable, KeyBinding, MouseButton, Role, ScrollHandle,
    SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions, actions,
    colors::{Colors, DefaultColors, GlobalColors},
    div,
    prelude::*,
    px, rems, size,
};
use log::info;
use mimalloc::MiMalloc;
use schemars::JsonSchema;
use serde::Deserialize;
use smallvec::{SmallVec, smallvec};

#[cfg(feature = "resources")]
use resources::Resources;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[cfg(not(debug_assertions))]
const DEFAULT_FONT: &str = ".IosevkaNavcon";

#[cfg(debug_assertions)]
const DEFAULT_FONT: &str = "Iosevka Term";

actions!(kawwosh, [Quit, TabNew, TabNext, TabPrev]);

#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = kawwosh)]
pub struct TabSelect {
    pub idx: usize,
}

#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = kawwosh)]
pub struct TabClose {
    pub idx: Option<usize>,
}

#[derive(Debug)]
struct Tab {
    idx: usize,
    focus_handle: FocusHandle,
    name: SharedString,
    selected: bool,
}

impl Tab {
    fn new(
        idx: usize,
        name: impl Into<SharedString>,
        focus_handle: FocusHandle,
        selected: bool,
    ) -> Self {
        Self {
            idx,
            focus_handle,
            name: name.into(),
            selected,
        }
    }
}

impl Render for Tab {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors();
        let text_size = rems(1.);
        let (bg, text_color) = match self.selected {
            false => (colors.container, colors.text),
            true => (colors.selected, colors.selected_text),
        };
        let idx = self.idx;
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
            .on_mouse_down(MouseButton::Right, move |_event, window, cx| {
                window.dispatch_action(TabClose { idx: Some(idx) }.boxed_clone(), cx);
            })
            .child(self.name.clone())
    }
}

impl Focusable for Tab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[derive(Debug)]
struct Kawwosh {
    focus_handle: FocusHandle,
    tabs_scroll: ScrollHandle,
    tabs: SmallVec<[Entity<Tab>; 4]>,
    last_index: usize,
    selected_tab: usize,
}

impl Kawwosh {
    fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_tab == index && index != 0 {
            return;
        }

        self.selected_tab = if index == self.tabs.len() {
            0
        } else {
            index.min(self.tabs.len().saturating_sub(1))
        };
        for (i, tab) in self.tabs.iter().enumerate() {
            tab.update(cx, |tab, cx| {
                tab.idx = i;
                tab.selected = i == self.selected_tab;
                if tab.selected {
                    self.tabs_scroll.scroll_to_top_of_item(i);
                    tab.focus_handle.focus(window, cx);
                }
            });
        }
    }
}

impl Render for Kawwosh {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let colors = cx.default_colors();
        div()
            .id("root")
            .track_focus(&self.focus_handle(cx))
            .on_action(
                cx.listener(|this: &mut Self, action: &TabSelect, window, cx| {
                    this.select_tab(action.idx, window, cx);
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|this: &mut Self, _: &TabNext, window, cx| {
                this.select_tab(this.selected_tab.wrapping_add(1), window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this: &mut Self, _: &TabPrev, window, cx| {
                this.select_tab(this.selected_tab.wrapping_sub(1), window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this: &mut Self, _: &TabNew, window, cx| {
                this.last_index += 1;
                this.tabs.push(cx.new(|_| {
                    let id = this.last_index;
                    Tab::new(id, format!("Tab {id}"), this.focus_handle.clone(), true)
                }));
                this.select_tab(this.tabs.len() - 1, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, e: &TabClose, window, cx| {
                if this.tabs.len() == 1 {
                    window.dispatch_action(Quit.boxed_clone(), cx);
                } else {
                    let idx = e.idx.unwrap_or(this.selected_tab);
                    this.tabs.remove(idx);
                    if idx <= this.selected_tab {
                        this.select_tab(this.selected_tab.saturating_sub(1), window, cx);
                    }
                    cx.notify();
                }
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
                    .flex()
                    .flex_row()
                    .justify_evenly()
                    .overflow_scroll()
                    .role(Role::TabList)
                    .tab_group()
                    .track_scroll(&self.tabs_scroll)
                    .children(self.tabs.iter().map(|t| t.clone())),
            )
            .child(
                div().flex().flex_row().p_2().child(
                    div()
                        .id("iosevka-text")
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .max_h_10()
                        .p_2()
                        .border_1()
                        .rounded_lg()
                        .border_color(colors.selected)
                        .text_2xl()
                        .text_color(colors.text)
                        .hover(|s| s.bg(colors.selected).text_color(colors.selected_text))
                        .on_click(|_this, _w, _cx| {
                            info!("Click");
                        })
                        .child("Iosevka"),
                ),
            )
    }
}

impl Focusable for Kawwosh {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn create_window(cx: &mut gpui::App) -> Result<gpui::WindowHandle<Kawwosh>> {
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(920.), px(720.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("GPUI Typography".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            cx.set_global(GlobalColors(Arc::new(Colors::for_appearance(window))));
            let focus_handle = cx.focus_handle();
            let tabs = smallvec![cx.new(|_| Tab::new(0, "default", focus_handle.clone(), true))];
            let app = cx.new(|_cx| Kawwosh {
                focus_handle,
                tabs_scroll: ScrollHandle::new(),
                tabs,
                last_index: 0,
                selected_tab: 0,
            });
            app
        },
    )?;
    window.update(cx, |this, w, cx| {
        w.focus(&this.focus_handle, cx);
        cx.activate(true);
    })?;
    Ok(window)
}

fn main() -> Result<()> {
    env_logger::builder()
        .format_timestamp(None)
        .parse_env(Env::default().default_filter_or("trace"))
        .init();

    let app = gpui_platform::application();

    #[cfg(feature = "resources")]
    let app = app.with_assets(Resources);

    app.run(|cx| {
        #[cfg(all(feature = "resources", not(debug_assertions)))]
        Resources::load_fonts(cx).expect("Load static assets");

        create_window(cx).expect("Create window");

        cx.on_window_closed(|cx, _window_id| {
            cx.quit();
        })
        .detach();

        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("escape", Quit, None),
            KeyBinding::new("cmd-1", TabSelect { idx: 0 }, None),
            KeyBinding::new("cmd-2", TabSelect { idx: 1 }, None),
            KeyBinding::new("cmd-3", TabSelect { idx: 2 }, None),
            KeyBinding::new("cmd-4", TabSelect { idx: 3 }, None),
            KeyBinding::new("cmd-5", TabSelect { idx: 4 }, None),
            KeyBinding::new("cmd-6", TabSelect { idx: 5 }, None),
            KeyBinding::new("cmd-7", TabSelect { idx: 6 }, None),
            KeyBinding::new("cmd-8", TabSelect { idx: 7 }, None),
            KeyBinding::new("cmd-9", TabSelect { idx: 8 }, None),
            KeyBinding::new("cmd-n", TabNext, None),
            KeyBinding::new("cmd-p", TabPrev, None),
            KeyBinding::new("cmd-t", TabNew, None),
            KeyBinding::new("cmd-w", TabClose { idx: None }, None),
        ]);
        cx.on_action(|Quit, cx| cx.quit());
    });
    Ok(())
}
