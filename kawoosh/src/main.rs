use std::{ffi::CStr, sync::Mutex};

use derive_more::Debug;
use env_logger::Env;
use log::debug;
use sdl3::{
    Sdl, VideoSubsystem,
    event::Event,
    keyboard::Keycode,
    messagebox::{MessageBoxFlag, show_simple_message_box},
    pixels::Color,
    render::WindowCanvas,
    timer::ticks,
    video::WindowFlags,
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

#[derive(Debug)]
struct AppState {
    #[debug(ignore)]
    main: MainThreadData<SdlData>,
    title: String,
    title_bar_height: usize,
    title_bar_left_padding: usize,
    mx: f32,
    my: f32,
}

#[app_impl]
impl AppState {
    fn new() -> Result<Self, Box<dyn ::core::error::Error>> {
        unsafe { SDL_SetHint(SDL_HINT_MAIN_CALLBACK_RATE, c"0".as_ptr()) };
        let sdl = sdl3::init()?;
        let video = sdl.video()?;

        let title = "rust-sdl3 demo".to_string();
        let window = video
            .window(&title, 800, 600)
            .set_flags(WindowFlags::TRANSPARENT)
            .resizable()
            .position_centered()
            // .high_pixel_density()
            .build()?;

        #[allow(unused_assignments)]
        let mut title_bar_height: usize = 0;
        #[allow(unused_assignments)]
        let mut title_bar_left_padding: usize = 0;

        #[cfg(target_os = "macos")]
        {
            let (titlebar_top_inset, titlebar_right_inset) =
                sys_macos::hide_window_titlebar(window.raw());
            title_bar_left_padding = titlebar_right_inset;
            title_bar_height = titlebar_top_inset;
        }

        let canvas = window.into_canvas();
        let res = Self {
            main: MainThreadData::assert_new(SdlData {
                canvas,
                _video: video,
                _sdl: sdl,
            }),
            title,
            title_bar_height,
            title_bar_left_padding,
            mx: 0.0,
            my: 0.0,
        };
        debug!("{res:?}");
        Ok(res)
    }

    fn app_init() -> AppResultWithState<Box<Mutex<Self>>> {
        env_logger::builder()
            .parse_env(Env::default().default_filter_or("debug"))
            .format_timestamp(None)
            .init();
        match Self::new() {
            Ok(app) => {
                #[cfg(target_os = "macos")]
                sys_macos::install_common_mode_timer();
                AppResultWithState::Continue(Box::new(Mutex::new(app)))
            }
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
        canvas.set_draw_color(Color::WHITE);
        let _ = canvas.draw_debug_text(
            &self.title,
            (
                (self.title_bar_left_padding + 10) as i32,
                (self.title_bar_height / 2 - 4) as i32,
            ),
        );
        let _ = canvas.draw_debug_text(
            &format!("Callbacks running for {} ms", ticks()),
            (4, (self.title_bar_height + 4) as i32),
        );
        let _ = canvas.draw_debug_text(
            &format!("Mouse x: {}", self.mx),
            (4, (self.title_bar_height + 20) as i32),
        );
        let _ = canvas.draw_debug_text(
            &format!("      y: {}", self.my),
            (4, (self.title_bar_height + 28) as i32),
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
