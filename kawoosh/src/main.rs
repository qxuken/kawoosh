use std::sync::Arc;

use anyhow::Result;
use env_logger::Env;
use gpui::{
    Focusable, TitlebarOptions, Window, WindowBounds, WindowOptions,
    colors::{Colors, GlobalColors},
    px, size,
};
use mimalloc::MiMalloc;

#[cfg(feature = "resources")]
use resources::Resources;

use kawoosh::{Kawoosh, actions::*, default_keybinds};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

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

        cx.bind_keys(default_keybinds());
        cx.on_action(|Quit, cx| cx.quit());
    });
    Ok(())
}

fn set_global_colors(cx: &mut gpui::App, window: &mut Window) {
    cx.set_global(GlobalColors(Arc::new(Colors::for_appearance(window))));
}

fn create_window(cx: &mut gpui::App) -> Result<gpui::WindowHandle<Kawoosh>> {
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(920.), px(720.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("Kawoosh".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            set_global_colors(cx, window);
            window
                .observe_window_appearance(|window, cx| {
                    set_global_colors(cx, window);
                })
                .detach();
            let focus_handle = cx.focus_handle();
            Kawoosh::new(cx, focus_handle)
        },
    )?;
    window.update(cx, |this, w, cx| {
        let handle = this.tabs[0].read(cx).content.read(cx).focus_handle(cx);
        w.focus(&handle, cx);
        cx.activate(true);
    })?;
    Ok(window)
}
