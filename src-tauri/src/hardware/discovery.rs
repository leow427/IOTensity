/// Recreating the browser alone keeps its multicast sockets. macOS can leave
/// those sockets unable to receive after a permission or network transition.
/// Retry the entire daemon with bounded backoff when no verified device is online.
pub(super) struct Recovery {
    next_ms: u64,
    delay_ms: u64,
}

impl Default for Recovery {
    fn default() -> Self {
        Self {
            next_ms: 5_000,
            delay_ms: 5_000,
        }
    }
}

impl Recovery {
    pub fn restart(&mut self, now_ms: u64, requested: bool, online: bool) -> bool {
        if requested || online {
            self.delay_ms = 5_000;
            self.next_ms = now_ms.saturating_add(self.delay_ms);
            return requested;
        }
        if now_ms < self.next_ms {
            return false;
        }
        self.delay_ms = (self.delay_ms * 2).min(30_000);
        self.next_ms = now_ms.saturating_add(self.delay_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_silent_discovery_with_bounded_backoff() {
        let mut recovery = Recovery::default();
        for (now, expected) in [
            (0, false),
            (4_999, false),
            (5_000, true),
            (5_001, false),
            (14_999, false),
            (15_000, true),
            (35_000, true),
            (64_999, false),
            (65_000, true),
            (95_000, true),
        ] {
            assert_eq!(recovery.restart(now, false, false), expected, "{now}");
        }
    }

    #[test]
    fn healthy_devices_prevent_restarts_and_reset_failure_backoff() {
        let mut recovery = Recovery::default();
        assert!(recovery.restart(5_000, false, false));
        assert!(!recovery.restart(50_000, false, true));
        assert!(!recovery.restart(54_999, false, false));
        assert!(recovery.restart(55_000, false, false));
    }

    #[test]
    fn network_or_user_retry_does_not_wait_for_backoff() {
        let mut recovery = Recovery::default();
        assert!(recovery.restart(100, true, true));
        assert!(!recovery.restart(5_099, false, false));
        assert!(recovery.restart(5_100, false, false));
    }
}
