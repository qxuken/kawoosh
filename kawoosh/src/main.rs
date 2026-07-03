use std::{ffi::CStr, sync::Mutex};

use bytemuck::checked::cast_slice;
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use env_logger::Env;
use log::{debug, error, log_enabled, trace};
use sdl3::{
    Sdl, VideoSubsystem,
    event::{Event, WindowEvent},
    keyboard::Keycode,
    messagebox::{MessageBoxFlag, show_simple_message_box},
    pixels::Color,
    render::{FRect, WindowCanvas},
    timer::ticks,
};
use sdl3_main::{AppResult, AppResultWithState, MainThreadData, MainThreadToken, app_impl};
use sdl3_sys::everything::*;

#[cfg(target_os = "macos")]
mod sys_macos;

struct SdlData {
    canvas: WindowCanvas,
    _video: VideoSubsystem,
    _sdl: Sdl,
}

impl SdlData {
    fn canvas_mut(&mut self) -> &mut WindowCanvas {
        &mut self.canvas
    }
}

struct TextNode<'a> {
    text: String,
    color: cosmic_text::Color,
    metrics: Metrics,
    attr: Attrs<'a>,
    buffer: Buffer,
}

impl<'a> TextNode<'a> {
    fn make_title(
        font_system: &mut FontSystem,
        text: impl ToString,
        width_opt: Option<f32>,
        height_opt: Option<f32>,
    ) -> Self {
        let mut node = TextNode::new(
            text,
            cosmic_text::Color::rgb(0xFF, 0xFF, 0xFF),
            Metrics {
                font_size: 14.0,
                line_height: 16.0,
            },
        );
        node.set_size(width_opt, height_opt);
        node.reshape(font_system);
        node
    }
    fn make_default(
        font_system: &mut FontSystem,
        text: impl ToString,
        width_opt: Option<f32>,
        height_opt: Option<f32>,
    ) -> Self {
        let mut node = TextNode::new(
            text,
            cosmic_text::Color::rgb(0xFF, 0xFF, 0xFF),
            Metrics {
                font_size: 16.0,
                line_height: 18.0,
            },
        );
        node.set_size(width_opt, height_opt);
        node.reshape(font_system);
        node
    }

    fn new(text: impl ToString, color: cosmic_text::Color, metrics: Metrics) -> Self {
        let buffer = Buffer::new_empty(metrics);
        let attr = Attrs::new()
            .color(color)
            .family(Family::Name("Iosevka Navcon"));
        let mut node = Self {
            text: String::new(),
            color,
            metrics,
            attr,
            buffer,
        };
        node.set_text(text);
        node
    }

    fn set_size(&mut self, width_opt: Option<f32>, height_opt: Option<f32>) {
        self.buffer.set_size(width_opt, height_opt);
    }
    fn set_text(&mut self, text: impl ToString) {
        let str = text.to_string();
        self.buffer
            .set_text(&str, &self.attr, Shaping::Advanced, None);
        self.text = str;
    }
    fn reshape(&mut self, font_system: &mut FontSystem) {
        self.buffer.shape_until_scroll(font_system, false);
        let _ = self.buffer.layout_runs().count();
    }
    pub fn draw<F>(&mut self, font_system: &mut FontSystem, cache: &mut crate::SwashCache, mut f: F)
    where
        F: FnMut(Metrics, i32, i32, u32, u32, cosmic_text::Color),
    {
        self.buffer
            .borrow_with(font_system)
            .draw(cache, self.color, |r, g, b, a, color| {
                f(self.metrics, r, g, b, a, color)
            });
    }
}

struct AppState<'a> {
    main: MainThreadData<SdlData>,
    font_system: FontSystem,
    swash_cache: SwashCache,
    scale: f32,
    padding: f32,
    title_bar_height: f32,
    title_bar_left_padding: f32,
    title: TextNode<'a>,
    debug: TextNode<'a>,
    mouse_x: f32,
    mouse_y: f32,
}

impl<'a> AppState<'a> {
    fn update_scale(&mut self, scale: f32) {
        let rescale = scale / self.scale;
        #[inline]
        fn rescale_buffer(buf: &mut Buffer, new_scale: f32) {
            let metrics = buf.metrics();
            let (w, h) = buf.size();
            buf.set_metrics_and_size(
                metrics.scale(new_scale),
                w.map(|v| v * new_scale),
                h.map(|v| v * new_scale),
            );
        }
        self.title_bar_height *= rescale;
        self.title_bar_left_padding *= rescale;
        self.padding *= rescale;
        rescale_buffer(&mut self.title.buffer, rescale);
        self.title.reshape(&mut self.font_system);
        rescale_buffer(&mut self.debug.buffer, rescale);
        self.scale = scale;
        debug!("New scale {scale}");
    }
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
            .resizable()
            .position_centered()
            .high_pixel_density()
            .build()?;
        let scale = window.display_scale();

        let title_bar_height = 0.0;
        let title_bar_left_padding = 0.0;

        #[cfg(target_os = "macos")]
        {
            let (titlebar_top_inset, titlebar_right_inset) =
                sys_macos::hide_window_titlebar(window.raw());
            title_bar_left_padding = titlebar_right_inset;
            title_bar_height = titlebar_top_inset;
        }

        let canvas = window.into_canvas();
        #[cfg(target_os = "macos")]
        sys_macos::install_common_mode_timer();

        let viewport = canvas.viewport();

        let padding = 10.0;

        let title = TextNode::make_title(
            &mut font_system,
            title,
            Some(viewport.w as f32 / scale - title_bar_left_padding - padding * 2.0),
            Some(title_bar_height),
        );
        let debug = TextNode::make_default(
            &mut font_system,
            String::new(),
            Some(viewport.h as f32 / scale - title_bar_height - padding * 2.0),
            Some(viewport.w as f32 / scale - padding * 2.0),
        );

        let mut res = Self {
            main: MainThreadData::assert_new(SdlData {
                canvas,
                _video: video,
                _sdl: sdl,
            }),
            font_system,
            swash_cache,
            scale: 1.0,
            padding,
            title_bar_height,
            title_bar_left_padding,
            title,
            debug,
            mouse_x: 0.0,
            mouse_y: 0.0,
        };
        res.update_scale(scale);

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
                error!("{error_msg:?}");
                let _ = show_simple_message_box(MessageBoxFlag::ERROR, "Error!", &error_msg, None);
                AppResultWithState::Failure(None)
            }
        }
    }

    fn app_iterate(&mut self) -> AppResult {
        let Some(canvas) = MainThreadToken::get()
            .map(|token| self.main.get_mut(token))
            .map(SdlData::canvas_mut)
        else {
            return AppResult::Continue;
        };

        canvas.set_draw_color(Color::BLACK);
        canvas.clear();
        canvas.set_blend_mode(sdl3::render::BlendMode::Blend);

        self.debug.set_text(format!(
            "Callbacks running for {} ms\nMouse x: {}\n      y: {}",
            ticks(),
            self.mouse_x,
            self.mouse_y,
        ));
        self.debug.reshape(&mut self.font_system);
        self.debug.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            |_metrics, x, y, w, h, color| {
                canvas.set_draw_color(Color::RGBA(color.r(), color.g(), color.b(), color.a()));
                canvas
                    .fill_rect(FRect::new(
                        self.padding + x as f32,
                        self.title_bar_height + self.padding + y as f32,
                        w as f32,
                        h as f32,
                    ))
                    .unwrap();
            },
        );

        self.title.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            |metrics, x, y, w, h, color| {
                canvas.set_draw_color(Color::RGBA(color.r(), color.g(), color.b(), color.a()));
                canvas
                    .fill_rect(FRect::new(
                        self.title_bar_left_padding + self.padding + x as f32,
                        self.title_bar_height - metrics.line_height * (1.5 * self.scale) + y as f32,
                        w as f32,
                        h as f32,
                    ))
                    .unwrap();
            },
        );
        canvas.present();

        AppResult::Continue
    }

    fn app_event(&mut self, event: &SDL_Event) -> AppResult {
        if log_enabled!(log::Level::Debug) {
            let mut buf = [0i8; 256];
            let buf_n =
                unsafe { SDL_GetEventDescription(event, buf.as_mut_ptr(), buf.len() as i32) };
            let buf: &[u8] = cast_slice(&buf[0..(buf_n as usize)]);
            let desc = unsafe { CStr::from_bytes_with_nul_unchecked(buf) }.to_string_lossy();
            debug!("{desc}");
        }
        match Event::from_ll(*event) {
            Event::Quit { .. }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            } => AppResult::Success,
            Event::Window {
                win_event: WindowEvent::DisplayChanged(_),
                ..
            } => {
                let scale = {
                    let Some(canvas) = MainThreadToken::get()
                        .map(|token| self.main.get_mut(token))
                        .map(SdlData::canvas_mut)
                    else {
                        return AppResult::Continue;
                    };
                    canvas.window().display_scale()
                };
                self.update_scale(scale);
                return AppResult::Continue;
            }
            Event::Window {
                win_event: WindowEvent::Resized(w, h),
                ..
            } => {
                self.title.set_size(
                    Some(w as f32 * self.scale - self.padding * 2.0),
                    Some(self.title_bar_height),
                );
                self.title.reshape(&mut self.font_system);
                self.debug.set_size(
                    Some(w as f32 * self.scale - self.padding * 2.0),
                    Some(h as f32 * self.scale - self.title_bar_height - self.padding * 2.0),
                );
                AppResult::Continue
            }
            Event::MouseMotion { x, y, .. } => {
                self.mouse_x = x;
                self.mouse_y = y;
                AppResult::Continue
            }
            _ => AppResult::Continue,
        }
    }
}
