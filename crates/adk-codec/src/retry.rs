//! Legacy retry-policy arithmetic; does not install a native execution policy.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RetryBackoffSettings {
    pub initial_delay_ms: i64,
    pub max_delay_ms: i64,
    pub multiplier: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RetryPolicy {
    pub max_retries: i64,
    pub backoff: RetryBackoffSettings,
}

pub fn default_retry_policy() -> RetryPolicy {
    RetryPolicy {
        max_retries: 3,
        backoff: RetryBackoffSettings {
            initial_delay_ms: 1000,
            max_delay_ms: 30000,
            multiplier: 2.0,
        },
    }
}

impl RetryPolicy {
    /// Signed Go time.Duration nanoseconds, including 64-bit wrapping behavior.
    /// This is not a safe sleep duration; hosts must resolve it against their limits.
    pub fn delay_for_attempt(&self, attempt: i64) -> i64 {
        let initial = self.backoff.initial_delay_ms;
        if initial <= 0 {
            return 0;
        }
        if attempt <= 0 {
            return initial.wrapping_mul(1_000_000);
        }
        let multiplier = if self.backoff.multiplier < 1.0 {
            2.0
        } else {
            self.backoff.multiplier
        };
        let max_delay = if self.backoff.max_delay_ms <= 0 {
            30000
        } else {
            self.backoff.max_delay_ms
        };
        let mut delay = initial as f64 * multiplier.powf(attempt as f64);
        if delay > max_delay as f64 {
            delay = max_delay as f64;
        }
        // Go's pinned amd64 float-to-int conversion differs from Rust saturation.
        let millis = if !delay.is_finite() || delay >= 9_223_372_036_854_775_808.0 {
            i64::MIN
        } else {
            delay as i64
        };
        millis.wrapping_mul(1_000_000)
    }
}
