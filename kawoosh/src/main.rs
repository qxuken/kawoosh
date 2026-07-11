use std::sync::Arc;

use anyhow::Result;
use env_logger::Env;
use gpui::{
    KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, actions,
    colors::{Colors, DefaultColors, GlobalColors},
    div,
    prelude::*,
    px, size,
};
use log::info;
use mimalloc::MiMalloc;
#[cfg(feature = "resources")]
use resources::Resources;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const DEFAULT_FONT: &str = ".IosevkaNavcon";

actions!(main, [Quit]);

struct App;

impl Render for App {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let colors = cx.default_colors();
        div()
            .flex()
            .flex_row()
            .font_family(DEFAULT_FONT)
            .gap_4()
            .p_10()
            .w_full()
            .h_full()
            .bg(colors.background)
            .child(
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
            )
    }
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

        let window = cx
            .open_window(
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
                    cx.new(|_cx| App)
                },
            )
            .unwrap();
        window.update(cx, |_v, _w, cx| cx.activate(true)).unwrap();

        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("escape", Quit, None),
        ]);
        cx.on_action(|Quit, cx| cx.quit());
    });
    Ok(())
}
