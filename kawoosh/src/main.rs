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

const DEFAULT_FONT: &str = "Iosevka Navcon";

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
        let attr = Attrs::new().color(color).family(Family::Name(DEFAULT_FONT));
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
    fn set_text_groups<'s, 'r, I>(&mut self, i: I)
    where
        I: IntoIterator<Item = (&'s str, Attrs<'r>)>,
    {
        self.buffer
            .set_rich_text(i, &self.attr, Shaping::Advanced, None);
    }
    fn reshape(&mut self, font_system: &mut FontSystem) {
        self.buffer.shape_until_scroll(font_system, false);
        // let _ = self.buffer.layout_runs().count();
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
        self.padding *= rescale;
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

        let canvas = window.into_canvas();
        #[cfg(target_os = "macos")]
        sys_macos::install_common_mode_timer();

        let viewport = canvas.viewport();

        let padding = 10.0;

        let debug = TextNode::make_default(
            &mut font_system,
            String::new(),
            Some(viewport.h as f32 / scale - padding * 2.0),
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

        let text = self.debug.attr.clone();
        let value = Attrs::new()
            .color(cosmic_text::Color::rgb(69, 129, 20))
            .family(Family::Name(DEFAULT_FONT));
        self.debug.set_text_groups([
            ("Callbacks running for ", text.clone()),
            (&ticks().to_string(), value.clone()),
            (" ms", value.clone()),
            ("\nMouse x: ", text.clone()),
            (&self.mouse_x.to_string(), value.clone()),
            ("\n      y: ", text),
            (&self.mouse_y.to_string(), value.clone()),
        ]);
        self.debug.reshape(&mut self.font_system);
        self.debug.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            |_metrics, x, y, w, h, color| {
                canvas.set_draw_color(Color::RGBA(color.r(), color.g(), color.b(), color.a()));
                canvas
                    .fill_rect(FRect::new(
                        self.padding + x as f32,
                        self.padding + y as f32,
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
                self.debug.set_size(
                    Some(w as f32 * self.scale - self.padding * 2.0),
                    Some(h as f32 * self.scale - self.padding * 2.0),
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
