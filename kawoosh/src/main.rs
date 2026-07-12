use std::sync::Arc;

use anyhow::Result;
use env_logger::Env;
use gpui::{
    Entity, KeyBinding, SharedString, TitlebarOptions, WindowBounds, WindowOptions, actions,
    colors::{Colors, DefaultColors, GlobalColors},
    div,
    prelude::*,
    px, size,
};
use log::info;
use mimalloc::MiMalloc;
#[cfg(feature = "resources")]
use resources::Resources;
use smallvec::SmallVec;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const DEFAULT_FONT: &str = ".IosevkaNavcon";

actions!(main, [Quit]);

struct Tab {
    name: SharedString,
}

impl Tab {
    fn new(name: SharedString) -> Self {
        Self { name }
    }
}

impl Render for Tab {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.default_colors();
        div()
            .id(("tab", cx.entity_id()))
            .bg(colors.container)
            .text_color(colors.text)
            .p_1()
            .hover(|s| s.bg(colors.selected).text_color(colors.selected_text))
            .child(self.name.clone())
    }
}

#[derive(Default)]
struct App {
    tabs: SmallVec<[Entity<Tab>; 4]>,
}

impl Render for App {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let colors = cx.default_colors();
        div()
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
                    .gap_1()
                    .children(self.tabs.iter().map(|t| t.clone()))
                    .child(
                        div()
                            .id("new_tab")
                            .text_color(colors.text)
                            .text_xl()
                            .p_1()
                            .hover(|s| s.bg(colors.selected).text_color(colors.selected_text))
                            .child("+"),
                    ),
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

fn create_window(cx: &mut gpui::App) -> Result<gpui::WindowHandle<App>> {
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
            let app = cx.new(|_cx| App::default());
            app.update(cx, |this, cx| {
                this.tabs.push(cx.new(|_| Tab::new("first_tab".into())));
            });
            app
        },
    )?;
    cx.activate(true);
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
        #[cfg(feature = "resources")]
        Resources::load_fonts(cx).expect("Load static assets");

        create_window(cx).expect("Create window");

        cx.on_window_closed(|cx, _window_id| {
            cx.quit();
        })
        .detach();

        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("escape", Quit, None),
        ]);
        cx.on_action(|Quit, cx| cx.quit());
    });
    Ok(())
}
