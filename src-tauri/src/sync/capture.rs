use super::processing::AnalysisImage;
use std::ffi::{c_char, c_void, CStr};

unsafe extern "C" {
    fn io_capture_start() -> *mut c_void;
    fn io_capture_stop(handle: *mut c_void);
    fn io_capture_state(handle: *mut c_void, message: *mut c_char, capacity: usize) -> i32;
    fn io_capture_frame(
        handle: *mut c_void,
        context: *mut c_void,
        receive: extern "C" fn(*mut c_void, *const u8, usize, usize, usize),
    );
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
    pub fn take_latest(&self) -> Option<Result<AnalysisImage, String>> {
        extern "C" fn receive(
            context: *mut c_void,
            bytes: *const u8,
            width: usize,
            height: usize,
            stride: usize,
        ) {
            // The native wrapper holds a locked retained buffer until this callback
            // returns. No borrowed pointer escapes and no pixels cross Tauri IPC.
            if bytes.is_null() || stride.checked_mul(height).is_none() {
                return;
            }
            let result = unsafe { &mut *(context as *mut Option<Result<AnalysisImage, String>>) };
            let pixels = unsafe { std::slice::from_raw_parts(bytes, stride * height) };
            *result = Some(AnalysisImage::from_bgra(pixels, width, height, stride));
        }
        let mut result: Option<Result<AnalysisImage, String>> = None;
        unsafe { io_capture_frame(self.0, &mut result as *mut _ as *mut c_void, receive) };
        result
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe { io_capture_release(self.0) }
    }
}
