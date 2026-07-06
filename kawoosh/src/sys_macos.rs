use std::ffi::c_void;

use log::trace;
use objc2_core_foundation::{
    CFAllocator, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext, CFString,
};

unsafe extern "C-unwind" fn common_mode_timer(_timer: *mut CFRunLoopTimer, _info: *mut c_void) {
    unsafe extern "C" {
        fn SDL_IterateMainCallbacks(pump_events: bool);
    }

    trace!("common_mode_timer:fire");
    unsafe { SDL_IterateMainCallbacks(false) };
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
