use std::ffi::c_void;

use log::trace;
use objc2::msg_send;
use objc2::runtime::NSObject;
use objc2_core_foundation::{
    CFAllocator, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext, CFString, CGRect,
};
use sdl3_sys::properties::SDL_GetPointerProperty;
use sdl3_sys::video::{SDL_GetWindowProperties, SDL_PROP_WINDOW_COCOA_WINDOW_POINTER};

unsafe extern "C-unwind" fn common_mode_timer(_timer: *mut CFRunLoopTimer, _info: *mut c_void) {
    unsafe extern "C" {
        fn SDL_IterateMainCallbacks(pump_events: bool);
    }

    trace!("common_mode_timer");
    unsafe { SDL_IterateMainCallbacks(true) };
}

pub fn install_common_mode_timer() {
    let Some(allocator) = CFAllocator::default() else {
        return;
    };
    let mut ctx = CFRunLoopTimerContext {
        version: 0,
        info: std::ptr::null_mut(),
        retain: None,
        release: None,
        copyDescription: None,
    };
    let timer = unsafe {
        CFRunLoopTimer::new(
            Some(&allocator),
            0.0,
            1.0 / 120.0,
            0,
            0,
            Some(common_mode_timer),
            &mut ctx,
        )
    };
    let Some(timer) = timer else {
        return;
    };
    let Some(run_loop) = CFRunLoop::main() else {
        return;
    };
    let tracking_mode = CFString::from_static_str("NSEventTrackingRunLoopMode");
    run_loop.add_timer(Some(&timer), Some(&tracking_mode));
}

pub fn hide_window_titlebar(sdl_window: *mut sdl3_sys::video::SDL_Window) -> (usize, usize) {
    unsafe {
        let props = SDL_GetWindowProperties(sdl_window);
        let ns_window = SDL_GetPointerProperty(
            props,
            SDL_PROP_WINDOW_COCOA_WINDOW_POINTER,
            std::ptr::null_mut(),
        );
        if ns_window.is_null() {
            return (0, 0);
        }
        let ns_window = ns_window as *mut NSObject;

        let _: () = msg_send![ns_window, setTitlebarAppearsTransparent: true];

        let title_hidden: i64 = 1;
        let _: () = msg_send![ns_window, setTitleVisibility: title_hidden];

        let style_mask: u64 = msg_send![ns_window, styleMask];
        let _: () = msg_send![ns_window, setStyleMask: style_mask | (1 << 15)];

        let frame: CGRect = msg_send![ns_window, frame];
        let content_layout: CGRect = msg_send![ns_window, contentLayoutRect];

        let top_inset = frame.size.height - (content_layout.origin.y + content_layout.size.height);
        //
        // Get the traffic lights' right offset by querying the zoom button
        // (the rightmost button in the cluster: close=0, minimize=1, zoom=2)
        let zoom_button: *mut NSObject = msg_send![ns_window, standardWindowButton: 2i64];

        let traffic_lights_right = if !zoom_button.is_null() {
            // Use bounds (in the button's own coordinate system, origin 0,0)
            // and convert to window coordinates (toView: nil)
            let zoom_bounds: CGRect = msg_send![zoom_button, bounds];
            let zoom_in_window: CGRect = msg_send![
                zoom_button,
                convertRect: zoom_bounds,
                toView: std::ptr::null_mut::<NSObject>()
            ];
            // Right edge of the zoom button, measured from the left edge of the window
            (zoom_in_window.origin.x + zoom_in_window.size.width).ceil() as usize
        } else {
            0
        };

        (top_inset.ceil() as usize, traffic_lights_right)
    }
}
