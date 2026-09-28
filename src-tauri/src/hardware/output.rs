use std::io;

/// Retry socket setup without stopping the scheduler or coupling UDP health to
/// discovery. The injected clock and binder exercise real recovery decisions.
pub(super) struct SocketRecovery<T> {
    pub socket: Option<T>,
    pub error: Option<String>,
    failures: u32,
    next_attempt: u64,
}
impl<T> Default for SocketRecovery<T> {
    fn default() -> Self {
        Self {
            socket: None,
            error: None,
            failures: 0,
            next_attempt: 0,
        }
    }
}
impl<T> SocketRecovery<T> {
    pub fn poll(&mut self, now: u64, bind: impl FnOnce() -> io::Result<T>) {
        if self.socket.is_some() || now < self.next_attempt {
            return;
        }
        match bind() {
            Ok(socket) => {
                self.socket = Some(socket);
                self.error = None;
                self.failures = 0;
            }
            Err(error) => {
                self.error = Some(format!("UDP output unavailable: {error}. Retrying…"));
                self.next_attempt = now + (500_u64 << self.failures.min(3));
                self.failures = self.failures.saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_setup_failure_retries_with_bounded_backoff_then_recovers() {
        let mut output = SocketRecovery::default();
        for now in [0, 500, 1500, 3500, 7500, 11500] {
            output.poll(now, || {
                Err(io::Error::from(io::ErrorKind::AddrNotAvailable))
            });
            assert!(output.error.is_some());
            output.poll(now + 1, || panic!("must respect retry deadline"));
        }
        output.poll(15500, || Ok("socket"));
        assert_eq!(output.socket, Some("socket"));
        assert!(output.error.is_none());
        output.poll(20000, || panic!("must retain healthy socket"));
    }
}
