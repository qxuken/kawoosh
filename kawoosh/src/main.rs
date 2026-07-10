use std::borrow::Cow;

use anyhow::Result;
use env_logger::Env;
use gpui::{
    AssetSource, KeyBinding, SharedString, TitlebarOptions, WindowBounds, WindowOptions, actions,
    colors::DefaultColors, div, prelude::*, px, size,
};
use rust_embed::Embed;

const DEFAULT_FONT: &str = "Iosevka Navcon";

#[derive(Embed)]
#[folder = "../assets/"]
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Assets::get(path).map(|r| r.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Assets::iter()
            .filter(|f| f.starts_with(path))
            .map(|f| SharedString::from(f))
            .collect())
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
            .font_family(DEFAULT_FONT)
            .flex()
            .flex_row()
            .gap_4()
            .p_10()
            .w_full()
            .h_full()
            .bg(colors.background)
            .child(
                div()
                    .id("iosevka-text")
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
        cx.text_system()
            .add_fonts(
                Assets::iter()
                    .filter(|f| f.starts_with("fonts/IosevkaNavcon") && f.ends_with(".ttf"))
                    .filter_map(|f| Assets::get(&f))
                    .map(|f| f.data)
                    .collect(),
            )
            .unwrap();
        cx.init_colors();
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("escape", Quit, None),
        ]);
        cx.on_action(|Quit, cx| cx.quit());
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
                |_window, cx| cx.new(|_cx| App),
            )
            .unwrap();
        window.update(cx, |_v, _w, cx| cx.activate(true)).unwrap();
    });
    Ok(())
}
