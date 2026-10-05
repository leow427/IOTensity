// A process activity assertion held only while native output is needed, so
// macOS App Nap cannot throttle output timing when IOTensity is hidden or
// occluded. Other platforms have no equivalent and use a no-op assertion.
#[cfg(target_os = "macos")]
struct Assertion(*mut std::ffi::c_void);
#[cfg(target_os = "macos")]
impl Assertion {
    fn begin() -> Self {
        unsafe extern "C" {
            fn io_activity_begin(reason: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        }
        Self(unsafe { io_activity_begin(c"Streaming colors to IOTensity lights".as_ptr()) })
    }
}
#[cfg(target_os = "macos")]
impl Drop for Assertion {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn io_activity_end(handle: *mut std::ffi::c_void);
        }
        unsafe { io_activity_end(self.0) }
    }
}
#[cfg(not(target_os = "macos"))]
struct Assertion;
#[cfg(not(target_os = "macos"))]
impl Assertion {
    fn begin() -> Self {
        Self
    }
}

/// Owned by one thread. Repeated `set` calls are idempotent; drop ends it.
#[derive(Default)]
pub struct Activity(Option<Assertion>);
impl Activity {
    pub fn set(&mut self, active: bool) {
        if !active {
            self.0 = None;
        } else if self.0.is_none() {
            self.0 = Some(Assertion::begin());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_begins_once_and_ends_when_idle() {
        let mut activity = Activity::default();
        assert!(activity.0.is_none());
        activity.set(true);
        #[cfg(target_os = "macos")]
        let token = activity.0.as_ref().unwrap().0;
        #[cfg(target_os = "macos")]
        assert!(!token.is_null());
        activity.set(true);
        #[cfg(target_os = "macos")]
        assert_eq!(activity.0.as_ref().unwrap().0, token); // No second assertion.
        assert!(activity.0.is_some());
        activity.set(false);
        activity.set(false);
        assert!(activity.0.is_none());
        activity.set(true);
        assert!(activity.0.is_some());
    }
}
