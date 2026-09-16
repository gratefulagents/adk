//! Scalar Go runner configuration. This is not a codec for callbacks or a full RunConfig.
use std::{num::NonZeroU32, time::Duration};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

fn null_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("configuration field {0} cannot be represented without changing semantics")]
pub struct ConfigError(pub &'static str);

/// Signed Go values are retained; resolving never overwrites zero/negative sentinels.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub struct RunConfigSentinels {
    #[serde(deserialize_with = "null_default")]
    pub max_turns: i64,
    #[serde(deserialize_with = "null_default")]
    pub sub_agent_max_turns: i64,
    #[serde(deserialize_with = "null_default")]
    pub max_concurrent_sub_agents: i64,
    #[serde(deserialize_with = "null_default")]
    pub consecutive_tool_error_limit: i64,
    #[serde(deserialize_with = "null_default")]
    pub stop_gate_max_blocks: i64,
    #[serde(deserialize_with = "null_default")]
    pub max_tool_output_bytes: i64,
    /// Go time.Duration is an integer number of nanoseconds, not seconds.
    #[serde(deserialize_with = "null_default")]
    pub model_call_timeout: i64,
    pub untrusted_tool_outputs: Option<bool>,
    pub tool_policy: Option<ToolPolicySentinels>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub struct ToolPolicySentinels {
    #[serde(deserialize_with = "null_default")]
    pub approval_required: bool,
    /// Seconds; nonpositive values do not install a timeout override.
    #[serde(deserialize_with = "null_default")]
    pub default_timeout: i64,
}

/// Values for runtime adapters. No native authorization policy is implicitly changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveRunConfig {
    pub max_turns: NonZeroU32,
    pub sub_agent_max_turns: NonZeroU32,
    pub max_concurrent_sub_agents: Option<usize>,
    pub consecutive_tool_error_limit: Option<usize>,
    pub stop_gate_max_blocks: usize,
    pub max_tool_output_bytes: Option<usize>,
    pub model_idle_timeout: Option<Duration>,
    pub untrusted_tool_outputs: bool,
    pub tool_policy: Option<EffectiveToolPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveToolPolicy {
    /// Go requires approval only for mutating, non-control-flow tools, not all tools.
    pub approval_required: bool,
    pub default_timeout: Option<Duration>,
}

fn positive_or(value: i64, default: i64) -> i64 {
    if value > 0 { value } else { default }
}

fn size(value: i64, field: &'static str) -> Result<usize, ConfigError> {
    usize::try_from(value).map_err(|_| ConfigError(field))
}

fn optional_size(
    value: i64,
    default: i64,
    field: &'static str,
) -> Result<Option<usize>, ConfigError> {
    if value < 0 {
        Ok(None)
    } else {
        size(if value == 0 { default } else { value }, field).map(Some)
    }
}

impl RunConfigSentinels {
    pub fn resolve(&self) -> Result<EffectiveRunConfig, ConfigError> {
        let turns = |value, default, field| {
            u32::try_from(positive_or(value, default))
                .ok()
                .and_then(NonZeroU32::new)
                .ok_or(ConfigError(field))
        };
        Ok(EffectiveRunConfig {
            max_turns: turns(self.max_turns, 100, "MaxTurns")?,
            sub_agent_max_turns: turns(self.sub_agent_max_turns, 50, "SubAgentMaxTurns")?,
            max_concurrent_sub_agents: if self.max_concurrent_sub_agents <= 0 {
                None
            } else {
                Some(size(
                    self.max_concurrent_sub_agents,
                    "MaxConcurrentSubAgents",
                )?)
            },
            consecutive_tool_error_limit: optional_size(
                self.consecutive_tool_error_limit,
                3,
                "ConsecutiveToolErrorLimit",
            )?,
            stop_gate_max_blocks: size(
                positive_or(self.stop_gate_max_blocks, 8),
                "StopGateMaxBlocks",
            )?,
            max_tool_output_bytes: optional_size(
                self.max_tool_output_bytes,
                16 * 1024,
                "MaxToolOutputBytes",
            )?,
            model_idle_timeout: match self.model_call_timeout {
                n if n < 0 => None,
                0 => Some(Duration::from_secs(300)),
                n => Some(Duration::from_nanos(n as u64)),
            },
            untrusted_tool_outputs: self.untrusted_tool_outputs.unwrap_or(true),
            tool_policy: self
                .tool_policy
                .as_ref()
                .map(|p| {
                    if p.default_timeout > i64::MAX / 1_000_000_000 {
                        return Err(ConfigError("DefaultTimeout"));
                    }
                    Ok(EffectiveToolPolicy {
                        approval_required: p.approval_required,
                        default_timeout: (p.default_timeout > 0)
                            .then(|| Duration::from_secs(p.default_timeout as u64)),
                    })
                })
                .transpose()?,
        })
    }

    /// Canonical explicit values, not the original spelling of a sentinel.
    /// Keep the original DTO when exact zero/negative/default round-trips are needed.
    pub fn from_effective(value: &EffectiveRunConfig) -> Result<Self, ConfigError> {
        let positive = |n: usize, field| {
            i64::try_from(n)
                .ok()
                .filter(|n| *n > 0)
                .ok_or(ConfigError(field))
        };
        let optional = |n: Option<usize>, disabled, field| match n {
            None => Ok(disabled),
            Some(n) => positive(n, field),
        };
        Ok(Self {
            max_turns: value.max_turns.get().into(),
            sub_agent_max_turns: value.sub_agent_max_turns.get().into(),
            max_concurrent_sub_agents: optional(
                value.max_concurrent_sub_agents,
                0,
                "MaxConcurrentSubAgents",
            )?,
            consecutive_tool_error_limit: optional(
                value.consecutive_tool_error_limit,
                -1,
                "ConsecutiveToolErrorLimit",
            )?,
            stop_gate_max_blocks: positive(value.stop_gate_max_blocks, "StopGateMaxBlocks")?,
            max_tool_output_bytes: optional(value.max_tool_output_bytes, -1, "MaxToolOutputBytes")?,
            model_call_timeout: match value.model_idle_timeout {
                None => -1,
                Some(d) => i64::try_from(d.as_nanos())
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or(ConfigError("ModelCallTimeout"))?,
            },
            untrusted_tool_outputs: Some(value.untrusted_tool_outputs),
            tool_policy: value
                .tool_policy
                .as_ref()
                .map(|p| {
                    let default_timeout = match p.default_timeout {
                        None => 0,
                        Some(d)
                            if d.subsec_nanos() == 0
                                && !d.is_zero()
                                && d.as_secs() <= (i64::MAX / 1_000_000_000) as u64 =>
                        {
                            d.as_secs() as i64
                        }
                        Some(_) => return Err(ConfigError("DefaultTimeout")),
                    };
                    Ok(ToolPolicySentinels {
                        approval_required: p.approval_required,
                        default_timeout,
                    })
                })
                .transpose()?,
        })
    }
}
