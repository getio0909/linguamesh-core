use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 限制路由候选数量，避免不受控的配置膨胀。
pub const MAX_ROUTING_CANDIDATES: usize = 32;
/// 限制路由配置标识和候选标识的长度。
pub const MAX_ROUTING_IDENTIFIER_BYTES: usize = 128;

/// 描述路由选择的运行模式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    /// 只使用用户明确指定的第一个候选。
    #[default]
    Manual,
    /// 按用户给出的顺序选择候选。
    Ordered,
    /// 按稳定的偏好规则计算候选排名。
    Automatic,
}

/// 描述自动路由的首要偏好。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingPreference {
    /// 使用稳定的标识排序作为唯一决胜规则。
    #[default]
    None,
    /// 优先使用本地候选。
    Local,
    /// 优先使用质量等级更高的候选。
    Quality,
    /// 优先使用预计延迟更低的候选。
    Latency,
    /// 优先使用预计成本更低的候选。
    Cost,
}

/// 描述路由选择需要满足的非秘密约束。
// 这些布尔字段对应彼此独立的安全和能力开关，保持序列化契约直观。
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoutingConstraints {
    /// 是否只允许本地候选。
    #[serde(default)]
    pub local_only: bool,
    /// 是否允许把内容发送给远程候选。
    #[serde(default = "default_true")]
    pub allow_remote: bool,
    /// 非空时只允许这些提供商标识。
    #[serde(default)]
    pub provider_allowlist: Vec<String>,
    /// 匹配这些提供商标识的候选会被拒绝。
    #[serde(default)]
    pub provider_denylist: Vec<String>,
    /// 非空时只允许这些模型标识。
    #[serde(default)]
    pub model_allowlist: Vec<String>,
    /// 匹配这些模型标识的候选会被拒绝。
    #[serde(default)]
    pub model_denylist: Vec<String>,
    /// 是否需要流式输出能力。
    #[serde(default)]
    pub require_streaming: bool,
    /// 是否需要文档翻译能力。
    #[serde(default)]
    pub require_document: bool,
    /// 可选的最低质量等级。
    #[serde(default)]
    pub minimum_quality_tier: Option<u8>,
    /// 可选的最大请求字节数。
    #[serde(default)]
    pub max_request_bytes: Option<usize>,
    /// 自动模式使用的稳定首要偏好。
    #[serde(default)]
    pub preference: RoutingPreference,
    /// 是否允许隐私敏感请求使用远程候选。
    #[serde(default)]
    pub privacy_sensitive: bool,
    /// 是否允许把合格候选作为显式回退链暴露给调用方。
    #[serde(default)]
    pub explicit_fallback_allowed: bool,
}

impl Default for RoutingConstraints {
    fn default() -> Self {
        Self {
            local_only: false,
            allow_remote: true,
            provider_allowlist: Vec::new(),
            provider_denylist: Vec::new(),
            model_allowlist: Vec::new(),
            model_denylist: Vec::new(),
            require_streaming: false,
            require_document: false,
            minimum_quality_tier: None,
            max_request_bytes: None,
            preference: RoutingPreference::default(),
            privacy_sensitive: false,
            explicit_fallback_allowed: false,
        }
    }
}

/// 描述一个不含端点、凭据或用户内容的路由候选。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoutingCandidate {
    /// 保存配置的稳定标识。
    pub provider_id: String,
    /// 提供商内的稳定模型标识。
    pub model_id: String,
    /// 候选是否指向本地服务。
    pub local: bool,
    /// 候选是否支持真实流式输出。
    pub supports_streaming: bool,
    /// 候选是否支持文档翻译。
    pub supports_document: bool,
    /// 候选可接受的近似请求字节容量。
    pub context_capacity_bytes: usize,
    /// 用户或目录提供的质量等级。
    pub quality_tier: u8,
    /// 可选的非权威延迟估计。
    pub estimated_latency_ms: Option<u64>,
    /// 可选的非权威成本估计，单位为微小货币单位。
    pub estimated_cost_micros: Option<u64>,
    /// 候选明确支持的源语言标签，空集合表示未知而非拒绝。
    #[serde(default)]
    pub source_locales: Vec<String>,
    /// 候选明确支持的目标语言标签，空集合表示未知而非拒绝。
    #[serde(default)]
    pub target_locales: Vec<String>,
}

impl RoutingCandidate {
    /// 创建经过标识和容量基础校验的候选。
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        local: bool,
        context_capacity_bytes: usize,
    ) -> Result<Self, RoutingError> {
        let provider_id = provider_id.into();
        let model_id = model_id.into();
        validate_identifier(&provider_id, "provider_id")?;
        validate_identifier(&model_id, "model_id")?;
        if context_capacity_bytes == 0 {
            return Err(RoutingError::InvalidConfiguration(
                "context capacity must be greater than zero",
            ));
        }
        Ok(Self {
            provider_id,
            model_id,
            local,
            supports_streaming: true,
            supports_document: false,
            context_capacity_bytes,
            quality_tier: 0,
            estimated_latency_ms: None,
            estimated_cost_micros: None,
            source_locales: Vec::new(),
            target_locales: Vec::new(),
        })
    }

    /// 返回不含秘密的稳定候选键。
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}@{}", self.provider_id, self.model_id)
    }
}

/// 描述一次路由请求的非秘密上下文。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoutingContext {
    /// 可选的源语言标签。
    pub source_locale: Option<String>,
    /// 必需的目标语言标签。
    pub target_locale: String,
    /// 请求正文的近似字节数。
    pub request_bytes: usize,
    /// 是否需要真实流式输出。
    pub require_streaming: bool,
    /// 是否需要文档能力。
    pub require_document: bool,
    /// 请求是否包含更严格的隐私敏感约束。
    pub privacy_sensitive: bool,
}

/// 描述一个被拒绝候选及其稳定原因。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingRejectionReason {
    /// 候选不在提供商允许列表中。
    ProviderNotAllowed,
    /// 候选命中提供商拒绝列表。
    ProviderDenied,
    /// 候选不在模型允许列表中。
    ModelNotAllowed,
    /// 候选命中模型拒绝列表。
    ModelDenied,
    /// 远程候选被本地模式拒绝。
    RemoteDisallowed,
    /// 隐私敏感请求不允许使用远程候选。
    PrivacyRemoteDisallowed,
    /// 候选不支持流式输出。
    StreamingUnsupported,
    /// 候选不支持文档翻译。
    DocumentUnsupported,
    /// 请求超过候选上下文容量。
    ContextTooSmall,
    /// 候选质量等级不足。
    QualityTooLow,
    /// 候选不支持源语言。
    SourceLocaleUnsupported,
    /// 候选不支持目标语言。
    TargetLocaleUnsupported,
    /// 请求超过路由配置的最大字节数。
    RequestTooLarge,
}

/// 描述被过滤的候选及原因。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoutingRejection {
    /// 被过滤的候选。
    pub candidate: RoutingCandidate,
    /// 稳定、可本地化的拒绝原因。
    pub reason: RoutingRejectionReason,
}

/// 描述自动路由的可审计排名输入。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoutingRank {
    /// 参与排名的候选。
    pub candidate: RoutingCandidate,
    /// 稳定的偏好比较分量，按偏好解释。
    pub score_components: Vec<i64>,
}

/// 描述一次成功的、可解释的路由选择。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// 最终选择的候选。
    pub selected: RoutingCandidate,
    /// 通过全部约束的候选，顺序由模式决定。
    pub eligible_candidates: Vec<RoutingCandidate>,
    /// 被过滤候选及其原因。
    pub rejected_candidates: Vec<RoutingRejection>,
    /// 自动模式使用的排名输入。
    pub ranking: Vec<RoutingRank>,
    /// 仅在显式允许回退时返回的候选顺序。
    pub fallback_order: Vec<RoutingCandidate>,
}

/// 描述一个持久化的非秘密路由配置。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoutingProfile {
    /// 配置稳定标识。
    pub id: String,
    /// 路由选择模式。
    #[serde(default)]
    pub mode: RoutingMode,
    /// 用户定义顺序或自动模式的候选集合。
    pub candidates: Vec<RoutingCandidate>,
    /// 路由约束和隐私策略。
    #[serde(default)]
    pub constraints: RoutingConstraints,
}

impl RoutingProfile {
    /// 创建并校验一个路由配置。
    pub fn new(
        id: impl Into<String>,
        mode: RoutingMode,
        candidates: Vec<RoutingCandidate>,
        constraints: RoutingConstraints,
    ) -> Result<Self, RoutingError> {
        let id = id.into();
        let profile = Self {
            id,
            mode,
            candidates,
            constraints,
        };
        profile.validate()?;
        Ok(profile)
    }

    /// 验证一个路由配置在持久化或选择前满足基础约束。
    pub fn validate(&self) -> Result<(), RoutingError> {
        validate_identifier(&self.id, "routing profile id")?;
        validate_candidates(&self.candidates)
    }

    /// 对非秘密请求上下文执行确定性、可解释的候选选择。
    pub fn select(&self, context: &RoutingContext) -> Result<RoutingDecision, RoutingError> {
        validate_candidates(&self.candidates)?;
        if context.target_locale.trim().is_empty() {
            return Err(RoutingError::InvalidConfiguration(
                "target locale must not be empty",
            ));
        }
        let mut eligible = Vec::new();
        let mut rejected = Vec::new();
        for candidate in &self.candidates {
            if let Some(reason) = self.rejection_reason(candidate, context) {
                rejected.push(RoutingRejection {
                    candidate: candidate.clone(),
                    reason,
                });
            } else {
                eligible.push(candidate.clone());
            }
        }
        if eligible.is_empty() {
            return Err(RoutingError::NoEligibleCandidates { rejected });
        }

        let ranking = if self.mode == RoutingMode::Automatic {
            let mut ranked = eligible
                .iter()
                .map(|candidate| RoutingRank {
                    candidate: candidate.clone(),
                    score_components: self.score_components(candidate),
                })
                .collect::<Vec<_>>();
            ranked.sort_by(|left, right| {
                right
                    .score_components
                    .cmp(&left.score_components)
                    .then_with(|| left.candidate.key().cmp(&right.candidate.key()))
            });
            eligible = ranked.iter().map(|rank| rank.candidate.clone()).collect();
            ranked
        } else {
            Vec::new()
        };

        let selected = eligible[0].clone();
        let fallback_order =
            if self.constraints.explicit_fallback_allowed && self.mode != RoutingMode::Manual {
                eligible.iter().skip(1).cloned().collect()
            } else {
                Vec::new()
            };
        Ok(RoutingDecision {
            selected,
            eligible_candidates: eligible,
            rejected_candidates: rejected,
            ranking,
            fallback_order,
        })
    }

    fn rejection_reason(
        &self,
        candidate: &RoutingCandidate,
        context: &RoutingContext,
    ) -> Option<RoutingRejectionReason> {
        let constraints = &self.constraints;
        if !constraints.provider_allowlist.is_empty()
            && !constraints
                .provider_allowlist
                .iter()
                .any(|id| id == &candidate.provider_id)
        {
            return Some(RoutingRejectionReason::ProviderNotAllowed);
        }
        if constraints
            .provider_denylist
            .iter()
            .any(|id| id == &candidate.provider_id)
        {
            return Some(RoutingRejectionReason::ProviderDenied);
        }
        if !constraints.model_allowlist.is_empty()
            && !constraints
                .model_allowlist
                .iter()
                .any(|id| id == &candidate.model_id)
        {
            return Some(RoutingRejectionReason::ModelNotAllowed);
        }
        if constraints
            .model_denylist
            .iter()
            .any(|id| id == &candidate.model_id)
        {
            return Some(RoutingRejectionReason::ModelDenied);
        }
        if constraints.local_only && !candidate.local {
            return Some(RoutingRejectionReason::RemoteDisallowed);
        }
        if (!constraints.allow_remote
            || (constraints.privacy_sensitive && context.privacy_sensitive))
            && !candidate.local
        {
            return Some(
                if constraints.privacy_sensitive && context.privacy_sensitive {
                    RoutingRejectionReason::PrivacyRemoteDisallowed
                } else {
                    RoutingRejectionReason::RemoteDisallowed
                },
            );
        }
        if (constraints.require_streaming || context.require_streaming)
            && !candidate.supports_streaming
        {
            return Some(RoutingRejectionReason::StreamingUnsupported);
        }
        if (constraints.require_document || context.require_document)
            && !candidate.supports_document
        {
            return Some(RoutingRejectionReason::DocumentUnsupported);
        }
        if context.request_bytes > candidate.context_capacity_bytes {
            return Some(RoutingRejectionReason::ContextTooSmall);
        }
        if constraints
            .max_request_bytes
            .is_some_and(|limit| context.request_bytes > limit)
        {
            return Some(RoutingRejectionReason::RequestTooLarge);
        }
        if constraints
            .minimum_quality_tier
            .is_some_and(|minimum| candidate.quality_tier < minimum)
        {
            return Some(RoutingRejectionReason::QualityTooLow);
        }
        if let Some(source_locale) = context.source_locale.as_deref()
            && !candidate.source_locales.is_empty()
            && !candidate
                .source_locales
                .iter()
                .any(|locale| locale.eq_ignore_ascii_case(source_locale))
        {
            return Some(RoutingRejectionReason::SourceLocaleUnsupported);
        }
        if !candidate.target_locales.is_empty()
            && !candidate
                .target_locales
                .iter()
                .any(|locale| locale.eq_ignore_ascii_case(&context.target_locale))
        {
            return Some(RoutingRejectionReason::TargetLocaleUnsupported);
        }
        None
    }

    fn score_components(&self, candidate: &RoutingCandidate) -> Vec<i64> {
        match self.constraints.preference {
            RoutingPreference::Local => vec![
                i64::from(candidate.local),
                i64::from(candidate.quality_tier),
            ],
            RoutingPreference::Quality => vec![
                i64::from(candidate.quality_tier),
                i64::from(candidate.local),
            ],
            RoutingPreference::Latency => vec![
                candidate
                    .estimated_latency_ms
                    .map_or(i64::MIN, |value| -i64::try_from(value).unwrap_or(i64::MAX)),
                i64::from(candidate.local),
            ],
            RoutingPreference::Cost => vec![
                candidate
                    .estimated_cost_micros
                    .map_or(i64::MIN, |value| -i64::try_from(value).unwrap_or(i64::MAX)),
                i64::from(candidate.local),
            ],
            RoutingPreference::None => vec![i64::from(candidate.local)],
        }
    }
}

/// 描述路由配置或选择失败。
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RoutingError {
    /// 配置不满足基础约束。
    #[error("Invalid routing configuration: {0}.")]
    InvalidConfiguration(&'static str),
    /// 没有候选满足全部约束。
    #[error("No routing candidate satisfies the configured constraints.")]
    NoEligibleCandidates { rejected: Vec<RoutingRejection> },
}

fn default_true() -> bool {
    true
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), RoutingError> {
    if value.is_empty()
        || value.len() > MAX_ROUTING_IDENTIFIER_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(RoutingError::InvalidConfiguration(field));
    }
    Ok(())
}

fn validate_candidates(candidates: &[RoutingCandidate]) -> Result<(), RoutingError> {
    if candidates.is_empty() {
        return Err(RoutingError::InvalidConfiguration(
            "at least one routing candidate is required",
        ));
    }
    if candidates.len() > MAX_ROUTING_CANDIDATES {
        return Err(RoutingError::InvalidConfiguration(
            "routing candidate count exceeds the limit",
        ));
    }
    for candidate in candidates {
        validate_identifier(&candidate.provider_id, "provider_id")?;
        validate_identifier(&candidate.model_id, "model_id")?;
        if candidate.context_capacity_bytes == 0 {
            return Err(RoutingError::InvalidConfiguration(
                "context capacity must be greater than zero",
            ));
        }
    }
    for (index, candidate) in candidates.iter().enumerate() {
        if candidates[..index].iter().any(|previous| {
            previous.provider_id == candidate.provider_id && previous.model_id == candidate.model_id
        }) {
            return Err(RoutingError::InvalidConfiguration(
                "routing candidates must have unique provider/model pairs",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        RoutingCandidate, RoutingConstraints, RoutingContext, RoutingError, RoutingMode,
        RoutingPreference, RoutingProfile, RoutingRejectionReason,
    };

    fn candidate(provider_id: &str, model_id: &str, local: bool) -> RoutingCandidate {
        RoutingCandidate::new(provider_id, model_id, local, 4096).expect("candidate")
    }

    #[test]
    fn ordered_mode_explains_rejections_and_exposes_explicit_fallback_order() {
        let mut local = candidate("local", "small", true);
        local.supports_streaming = false;
        let remote = candidate("remote", "large", false);
        let profile = RoutingProfile::new(
            "safe-chain",
            RoutingMode::Ordered,
            vec![local, remote.clone()],
            RoutingConstraints {
                require_streaming: true,
                explicit_fallback_allowed: true,
                ..RoutingConstraints::default()
            },
        )
        .expect("profile");
        let decision = profile
            .select(&RoutingContext {
                source_locale: Some("en".to_owned()),
                target_locale: "zh-CN".to_owned(),
                request_bytes: 32,
                require_streaming: false,
                require_document: false,
                privacy_sensitive: false,
            })
            .expect("decision");
        assert_eq!(decision.selected, remote);
        assert_eq!(decision.fallback_order, Vec::new());
        assert_eq!(decision.rejected_candidates.len(), 1);
        assert_eq!(
            decision.rejected_candidates[0].reason,
            RoutingRejectionReason::StreamingUnsupported
        );
    }

    #[test]
    fn automatic_mode_is_deterministic_and_prefer_quality_is_explainable() {
        let mut lower = candidate("provider-a", "model-a", true);
        lower.quality_tier = 1;
        let mut higher = candidate("provider-b", "model-b", false);
        higher.quality_tier = 3;
        let profile = RoutingProfile::new(
            "quality",
            RoutingMode::Automatic,
            vec![lower, higher.clone()],
            RoutingConstraints {
                preference: RoutingPreference::Quality,
                explicit_fallback_allowed: true,
                ..RoutingConstraints::default()
            },
        )
        .expect("profile");
        let context = RoutingContext {
            source_locale: None,
            target_locale: "zh-CN".to_owned(),
            request_bytes: 128,
            require_streaming: true,
            require_document: false,
            privacy_sensitive: false,
        };
        let first = profile.select(&context).expect("first decision");
        let second = profile.select(&context).expect("second decision");
        assert_eq!(first, second);
        assert_eq!(first.selected, higher);
        assert_eq!(first.ranking.len(), 2);
        assert_eq!(first.fallback_order.len(), 1);
    }

    #[test]
    fn privacy_sensitive_context_rejects_remote_candidates_without_explicit_permission() {
        let remote = candidate("remote", "model", false);
        let profile = RoutingProfile::new(
            "private",
            RoutingMode::Manual,
            vec![remote],
            RoutingConstraints {
                privacy_sensitive: true,
                ..RoutingConstraints::default()
            },
        )
        .expect("profile");
        let error = profile
            .select(&RoutingContext {
                source_locale: None,
                target_locale: "zh-CN".to_owned(),
                request_bytes: 16,
                require_streaming: true,
                require_document: false,
                privacy_sensitive: true,
            })
            .expect_err("remote privacy rejection");
        assert!(matches!(
            error,
            RoutingError::NoEligibleCandidates { rejected }
                if rejected[0].reason == RoutingRejectionReason::PrivacyRemoteDisallowed
        ));
    }

    #[test]
    fn duplicate_candidates_and_empty_profiles_are_rejected() {
        let first = candidate("provider", "model", true);
        assert!(matches!(
            RoutingProfile::new(
                "duplicate",
                RoutingMode::Ordered,
                vec![first.clone(), first],
                RoutingConstraints::default()
            ),
            Err(RoutingError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            RoutingProfile::new(
                "empty",
                RoutingMode::Manual,
                Vec::new(),
                RoutingConstraints::default()
            ),
            Err(RoutingError::InvalidConfiguration(_))
        ));
    }
}
