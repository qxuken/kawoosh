use std::{borrow::Cow, sync::Arc};

use anyhow::{Context, Result};
use env_logger::Env;
use gpui::{
    AssetSource, KeyBinding, SharedString, TitlebarOptions, WindowBounds, WindowOptions, actions,
    colors::{Colors, DefaultColors, GlobalColors},
    div,
    prelude::*,
    px, size,
};
use log::{debug, info, trace};
use rust_embed::Embed;

const DEFAULT_FONT: &str = "Iosevka Navcon";

#[derive(Embed)]
#[folder = "../assets/"]
#[exclude = "*.DS_Store"]
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Assets::get(path)
            .map(|r| Some(r.data))
            .with_context(|| format!("loading asset at path {path:?}"))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Assets::iter()
            .filter(|f| f.starts_with(path))
            .map(|f| f.into())
            .collect())
    }
}

impl Assets {
    fn load_fonts(cx: &mut gpui::App) -> Result<()> {
        trace!("Fonts::loading");
        let res = cx.text_system().add_fonts(
            Self::iter()
                .filter(|f| f.starts_with("fonts") && f.ends_with(".ttf"))
                .map(|f| Assets::get(&f).expect("Load from static storage"))
                .map(|f| f.data)
                .collect(),
        );
        debug!("Fonts::loaded");
        res
    }
}

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

    gpui_platform::application().with_assets(Assets).run(|cx| {
        Assets::load_fonts(cx).expect("Load static assets");

        let window = cx
            .open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("GPUI Typography".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::centered(size(px(920.), px(720.)), cx)),
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
