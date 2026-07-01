use std::{ffi::CStr, sync::Mutex};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use env_logger::Env;
use log::{debug, trace};
use sdl3::{
    Sdl, VideoSubsystem,
    event::Event,
    keyboard::Keycode,
    messagebox::{MessageBoxFlag, show_simple_message_box},
    pixels::Color,
    render::{FRect, WindowCanvas},
    timer::ticks,
    video::WindowFlags,
};
use sdl3_main::{AppResult, AppResultWithState, MainThreadData, MainThreadToken, app_impl};
use sdl3_sys::everything::*;

#[cfg(target_os = "macos")]
mod sys_macos;

const TITLE_COLOR: cosmic_text::Color = cosmic_text::Color::rgb(0xFF, 0xFF, 0xFF);
// const CONTENT_METRICS: Metrics = Metrics {
//     font_size: 16.0,
//     line_height: 18.0,
// };

struct SdlData {
    canvas: WindowCanvas,
    _video: VideoSubsystem,
    _sdl: Sdl,
}

struct AppState<'a> {
    main: MainThreadData<SdlData>,
    scale: f32,
    font_system: FontSystem,
    swash_cache: SwashCache,
    _title: String,
    title_metrics: Metrics,
    title_buffer: Buffer,
    _title_attr: Attrs<'a>,
    title_bar_height: f32,
    title_bar_left_padding: f32,
    mx: f32,
    my: f32,
}

#[app_impl]
impl AppState<'static> {
    fn new() -> Result<Self, Box<dyn ::core::error::Error>> {
        let mut font_system = FontSystem::new();
        let db = font_system.db_mut();
        db.load_fonts_dir("assets/fonts");
        let swash_cache = SwashCache::new();

        unsafe { SDL_SetHint(SDL_HINT_MAIN_CALLBACK_RATE, c"0".as_ptr()) };
        let sdl = sdl3::init()?;
        let video = sdl.video()?;

        let title = "rust-sdl3 demo".to_string();

        let window = video
            .window(&title, 800, 600)
            .set_flags(WindowFlags::TRANSPARENT)
            .resizable()
            .position_centered()
            .high_pixel_density()
            .build()?;
        let scale = window.display_scale();

        let title_bar_height;
        let title_bar_left_padding;

        #[cfg(target_os = "macos")]
        {
            let (titlebar_top_inset, titlebar_right_inset) =
                sys_macos::hide_window_titlebar(window.raw());
            title_bar_left_padding = titlebar_right_inset;
            title_bar_height = titlebar_top_inset;
        }

        let canvas = window.into_canvas();

        let title_metrics = Metrics {
            font_size: 14.0 * scale,
            line_height: 16.0 * scale,
        };
        let mut title_buffer = Buffer::new(&mut font_system, title_metrics);
        let title_attr = Attrs::new().family(Family::Name("IosevkaNavcon"));
        title_buffer.set_size(
            Some((800.0 - title_bar_left_padding) * scale),
            Some(title_bar_height * scale),
        );
        title_buffer.set_text(&title, &title_attr, Shaping::Advanced, None);
        title_buffer.shape_until_scroll(&mut font_system, true);
        let _ = title_buffer.layout_runs().count();

        #[cfg(target_os = "macos")]
        sys_macos::install_common_mode_timer();

        let res = Self {
            main: MainThreadData::assert_new(SdlData {
                canvas,
                _video: video,
                _sdl: sdl,
            }),
            scale,
            font_system,
            swash_cache,
            _title: title,
            title_metrics,
            title_buffer,
            _title_attr: title_attr,
            title_bar_height,
            title_bar_left_padding,
            mx: 0.0,
            my: 0.0,
        };

        trace!("{:?}", res.font_system);

        Ok(res)
    }

    fn app_init() -> AppResultWithState<Box<Mutex<Self>>> {
        env_logger::builder()
            .parse_env(Env::default().default_filter_or("debug"))
            .format_timestamp(None)
            .init();
        match Self::new() {
            Ok(app) => AppResultWithState::Continue(Box::new(Mutex::new(app))),
            Err(err) => {
                let error_msg = format!("Error initializing SDL: {err:?}");
                eprintln!("{error_msg}");
                let _ = show_simple_message_box(MessageBoxFlag::ERROR, "Error!", &error_msg, None);
                AppResultWithState::Failure(None)
            }
        }
    }

    fn app_iterate(&mut self) -> AppResult {
        let Some(token) = MainThreadToken::get() else {
            return AppResult::Continue;
        };
        let main = self.main.get_mut(token);
        let canvas = &mut main.canvas;

        canvas.set_draw_color(Color::BLACK);
        canvas.clear();
        let mut title_buffer = self.title_buffer.borrow_with(&mut self.font_system);
        title_buffer.draw(&mut self.swash_cache, TITLE_COLOR, |x, y, w, h, color| {
            canvas.set_draw_color(Color::RGBA(color.r(), color.g(), color.b(), color.a()));
            canvas
                .draw_rect(FRect::new(
                    (self.title_bar_left_padding + 10.0 + x as f32) * self.scale,
                    self.title_bar_height as f32 / 2.0 - self.title_metrics.line_height / 2.0
                        + y as f32 * self.scale,
                    w as f32,
                    h as f32,
                ))
                .unwrap();
        });
        canvas.set_draw_color(Color::WHITE);
        let _ = canvas.draw_debug_text(
            &format!("Callbacks running for {} ms", ticks()),
            (4.0 * self.scale, (self.title_bar_height + 4.0) * self.scale),
        );
        let _ = canvas.draw_debug_text(
            &format!("Mouse x: {}", self.mx),
            (
                4.0 * self.scale,
                (self.title_bar_height + 20.0) * self.scale,
            ),
        );
        let _ = canvas.draw_debug_text(
            &format!("      y: {}", self.my),
            (
                4.0 * self.scale,
                (self.title_bar_height + 28.0) * self.scale,
            ),
        );
        canvas.present();

        AppResult::Continue
    }

    fn app_event(&mut self, event: &SDL_Event) -> AppResult {
        let mut buf = [0i8; 256];
        unsafe { SDL_GetEventDescription(event, buf.as_mut_ptr(), buf.len() as i32) };
        let desc = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
        debug!("{desc}");
        match Event::from_ll(*event) {
            Event::Quit { .. }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            } => AppResult::Success,
            Event::MouseMotion { x, y, .. } => {
                self.mx = x;
                self.my = y;
                AppResult::Continue
            }
            _ => AppResult::Continue,
        }
    }
}
