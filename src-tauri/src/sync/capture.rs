use super::{processing::AnalysisImage, DisplayUpdate};
use std::ffi::{c_char, c_void, CStr};
use std::time::Duration;

unsafe extern "C" {
    fn io_capture_start() -> *mut c_void;
    fn io_capture_stop(handle: *mut c_void);
    fn io_capture_state(handle: *mut c_void, message: *mut c_char, capacity: usize) -> i32;
    fn io_capture_poll(
        handle: *mut c_void,
        message: *mut c_char,
        capacity: usize,
        status: *mut i32,
        age: *mut f64,
        context: *mut c_void,
        receive: extern "C" fn(*mut c_void, *const u8, usize, usize, usize),
    ) -> i32;
    fn io_capture_release(handle: *mut c_void);
}

// Created, polled and dropped on the single native output thread. The opaque
// Objective-C object synchronizes callbacks and owns all CVPixelBuffer lifetimes.
pub struct Capture(*mut c_void);
impl Capture {
    pub fn start() -> Self {
        Self(unsafe { io_capture_start() })
    }
    pub fn stop(&self) {
        unsafe { io_capture_stop(self.0) }
    }
    pub fn state(&self) -> (i32, String) {
        let mut message = [0 as c_char; 1024];
        let state = unsafe { io_capture_state(self.0, message.as_mut_ptr(), message.len()) };
        let message = unsafe { CStr::from_ptr(message.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        (state, message)
    }
    pub fn poll(&self) -> DisplayUpdate {
        extern "C" fn receive(
            context: *mut c_void,
            bytes: *const u8,
            width: usize,
            height: usize,
            stride: usize,
        ) {
            // The native wrapper holds a locked retained buffer until this callback
            // returns. No borrowed pointer escapes and no pixels cross Tauri IPC.
            let result = unsafe { &mut *(context as *mut Option<Result<AnalysisImage, String>>) };
            if bytes.is_null() || stride.checked_mul(height).is_none() {
                *result = Some(Err("Could not read the captured display frame.".into()));
                return;
            }
            let pixels = unsafe { std::slice::from_raw_parts(bytes, stride * height) };
            *result = Some(AnalysisImage::from_bgra(pixels, width, height, stride));
        }
        let mut result: Option<Result<AnalysisImage, String>> = None;
        let mut message = [0 as c_char; 1024];
        let mut status = -1;
        let mut age = -1.0;
        let state = unsafe {
            io_capture_poll(
                self.0,
                message.as_mut_ptr(),
                message.len(),
                &mut status,
                &mut age,
                &mut result as *mut _ as *mut c_void,
                receive,
            )
        };
        DisplayUpdate {
            state,
            message: unsafe { CStr::from_ptr(message.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
            frame_status: status,
            activity_age: Duration::try_from_secs_f64(age).ok(),
            frame: result,
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe { io_capture_release(self.0) }
    }
}
