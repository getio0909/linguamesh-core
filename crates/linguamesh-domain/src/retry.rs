use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

/// 默认退避初始等待时间，保持网络失败的首次回退短且可取消。
pub const DEFAULT_RETRY_BASE_DELAY_MS: u64 = 100;
/// 默认退避最大等待时间，避免候选切换形成无界等待。
pub const DEFAULT_RETRY_MAX_BACKOFF_MS: u64 = 8_000;
/// 默认稳定抖动比例上限。
pub const DEFAULT_RETRY_JITTER_PERCENT: u8 = 50;
/// 默认连续瞬时失败次数阈值。
pub const DEFAULT_RETRY_CIRCUIT_FAILURE_THRESHOLD: u32 = 2;
/// 默认熔断冷却时间。
pub const DEFAULT_RETRY_CIRCUIT_COOLDOWN_MS: u64 = 30_000;

/// 描述跨客户端共享的有界回退和熔断参数。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RetryPolicy {
    base_delay_ms: u64,
    max_backoff_ms: u64,
    jitter_percent: u8,
    circuit_failure_threshold: u32,
    circuit_cooldown_ms: u64,
    respect_retry_after: bool,
}

impl<'de> Deserialize<'de> for RetryPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RetryPolicyFields {
            base_delay_ms: u64,
            max_backoff_ms: u64,
            jitter_percent: u8,
            circuit_failure_threshold: u32,
            circuit_cooldown_ms: u64,
            respect_retry_after: bool,
        }

        let fields = RetryPolicyFields::deserialize(deserializer)?;
        Self::new(
            fields.base_delay_ms,
            fields.max_backoff_ms,
            fields.jitter_percent,
            fields.circuit_failure_threshold,
            fields.circuit_cooldown_ms,
            fields.respect_retry_after,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl RetryPolicy {
    /// 创建经过边界校验的策略，避免配置把等待或熔断状态扩大到不可控范围。
    pub fn new(
        base_delay_ms: u64,
        max_backoff_ms: u64,
        jitter_percent: u8,
        circuit_failure_threshold: u32,
        circuit_cooldown_ms: u64,
        respect_retry_after: bool,
    ) -> Result<Self, RetryPolicyError> {
        if base_delay_ms == 0 || base_delay_ms > max_backoff_ms {
            return Err(RetryPolicyError::InvalidDelayBounds);
        }
        if max_backoff_ms > 60_000 {
            return Err(RetryPolicyError::MaxBackoffTooLarge);
        }
        if jitter_percent > 100 {
            return Err(RetryPolicyError::JitterOutOfRange);
        }
        if !(1..=32).contains(&circuit_failure_threshold) {
            return Err(RetryPolicyError::CircuitThresholdOutOfRange);
        }
        if circuit_cooldown_ms == 0 || circuit_cooldown_ms > 300_000 {
            return Err(RetryPolicyError::CircuitCooldownOutOfRange);
        }
        Ok(Self {
            base_delay_ms,
            max_backoff_ms,
            jitter_percent,
            circuit_failure_threshold,
            circuit_cooldown_ms,
            respect_retry_after,
        })
    }

    /// 返回生产客户端共用的默认策略。
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            base_delay_ms: DEFAULT_RETRY_BASE_DELAY_MS,
            max_backoff_ms: DEFAULT_RETRY_MAX_BACKOFF_MS,
            jitter_percent: DEFAULT_RETRY_JITTER_PERCENT,
            circuit_failure_threshold: DEFAULT_RETRY_CIRCUIT_FAILURE_THRESHOLD,
            circuit_cooldown_ms: DEFAULT_RETRY_CIRCUIT_COOLDOWN_MS,
            respect_retry_after: true,
        }
    }

    /// 返回指数退避的初始等待时间。
    #[must_use]
    pub const fn base_delay_ms(self) -> u64 {
        self.base_delay_ms
    }

    /// 返回退避等待上限。
    #[must_use]
    pub const fn max_backoff_ms(self) -> u64 {
        self.max_backoff_ms
    }

    /// 返回稳定抖动比例。
    #[must_use]
    pub const fn jitter_percent(self) -> u8 {
        self.jitter_percent
    }

    /// 返回打开熔断所需的连续失败次数。
    #[must_use]
    pub const fn circuit_failure_threshold(self) -> u32 {
        self.circuit_failure_threshold
    }

    /// 返回熔断冷却时间。
    #[must_use]
    pub const fn circuit_cooldown_ms(self) -> u64 {
        self.circuit_cooldown_ms
    }

    /// 返回是否应使用提供商声明的 Retry-After 提示。
    #[must_use]
    pub const fn respect_retry_after(self) -> bool {
        self.respect_retry_after
    }

    /// 将提供商重试提示限制在策略允许的等待范围内。
    #[must_use]
    pub const fn bounded_retry_after_ms(self, retry_after_ms: Option<u64>) -> Option<u64> {
        if self.respect_retry_after {
            match retry_after_ms {
                Some(value) => {
                    if value > self.max_backoff_ms {
                        Some(self.max_backoff_ms)
                    } else {
                        Some(value)
                    }
                }
                None => None,
            }
        } else {
            None
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

/// 描述 `RetryPolicy` 配置越过安全边界的原因。
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RetryPolicyError {
    /// 初始等待必须为正数且不大于最大等待。
    #[error("retry delay bounds are invalid")]
    InvalidDelayBounds,
    /// 最大退避不能超过一分钟。
    #[error("retry maximum backoff is too large")]
    MaxBackoffTooLarge,
    /// 抖动比例必须位于 0 到 100 之间。
    #[error("retry jitter percentage is out of range")]
    JitterOutOfRange,
    /// 熔断失败阈值必须位于 1 到 32 之间。
    #[error("retry circuit threshold is out of range")]
    CircuitThresholdOutOfRange,
    /// 熔断冷却必须为正数且不超过五分钟。
    #[error("retry circuit cooldown is out of range")]
    CircuitCooldownOutOfRange,
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_RETRY_BASE_DELAY_MS, DEFAULT_RETRY_CIRCUIT_COOLDOWN_MS,
        DEFAULT_RETRY_CIRCUIT_FAILURE_THRESHOLD, DEFAULT_RETRY_JITTER_PERCENT,
        DEFAULT_RETRY_MAX_BACKOFF_MS, RetryPolicy, RetryPolicyError,
    };

    #[test]
    fn standard_policy_is_bounded_and_uses_retry_hints() {
        let policy = RetryPolicy::standard();
        assert_eq!(policy.base_delay_ms(), DEFAULT_RETRY_BASE_DELAY_MS);
        assert_eq!(policy.max_backoff_ms(), DEFAULT_RETRY_MAX_BACKOFF_MS);
        assert_eq!(policy.jitter_percent(), DEFAULT_RETRY_JITTER_PERCENT);
        assert_eq!(
            policy.circuit_failure_threshold(),
            DEFAULT_RETRY_CIRCUIT_FAILURE_THRESHOLD
        );
        assert_eq!(
            policy.circuit_cooldown_ms(),
            DEFAULT_RETRY_CIRCUIT_COOLDOWN_MS
        );
        assert_eq!(policy.bounded_retry_after_ms(Some(60_000)), Some(8_000));
    }

    #[test]
    fn policy_rejects_unbounded_values() {
        assert_eq!(
            RetryPolicy::new(0, 8_000, 50, 2, 30_000, true),
            Err(RetryPolicyError::InvalidDelayBounds)
        );
        assert_eq!(
            RetryPolicy::new(100, 60_001, 50, 2, 30_000, true),
            Err(RetryPolicyError::MaxBackoffTooLarge)
        );
        assert_eq!(
            RetryPolicy::new(100, 8_000, 101, 2, 30_000, true),
            Err(RetryPolicyError::JitterOutOfRange)
        );
        assert_eq!(
            RetryPolicy::new(100, 8_000, 50, 0, 30_000, true),
            Err(RetryPolicyError::CircuitThresholdOutOfRange)
        );
        assert_eq!(
            RetryPolicy::new(100, 8_000, 50, 2, 300_001, true),
            Err(RetryPolicyError::CircuitCooldownOutOfRange)
        );
    }

    #[test]
    fn policy_can_disable_provider_hints_without_changing_bounds() {
        let policy = RetryPolicy::new(100, 2_000, 25, 3, 10_000, false).expect("policy");
        assert!(!policy.respect_retry_after());
        assert_eq!(policy.bounded_retry_after_ms(Some(1_000)), None);
        assert_eq!(policy.max_backoff_ms(), 2_000);
    }

    #[test]
    fn deserialization_reuses_constructor_bounds() {
        let encoded = serde_json::json!({
            "base_delay_ms": 100,
            "max_backoff_ms": 60001,
            "jitter_percent": 50,
            "circuit_failure_threshold": 2,
            "circuit_cooldown_ms": 30000,
            "respect_retry_after": true,
        });

        let result = serde_json::from_value::<RetryPolicy>(encoded);

        assert!(result.is_err());
    }

    #[test]
    fn serialization_round_trip_preserves_valid_policy() {
        let policy = RetryPolicy::new(100, 2_000, 25, 3, 10_000, false).expect("policy");
        let encoded = serde_json::to_value(policy).expect("serialize policy");
        let decoded = serde_json::from_value::<RetryPolicy>(encoded).expect("deserialize policy");

        assert_eq!(decoded, policy);
    }
}
