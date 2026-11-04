//! 设置与 Provider 配置类型（settings.schema.json + provider-config-delivery.md）。
//! API Key 永不出现在这些类型的公开视图里；仅 `key_present` 布尔。

use crate::contracts::domain::Cadence;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// 超时预算（秒）。默认 15/180/240；组合约束 total >= connect + response。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeouts {
    pub connect_seconds: u32,
    pub response_seconds: u32,
    pub total_seconds: u32,
}

pub const DEFAULT_TIMEOUTS: Timeouts = Timeouts {
    connect_seconds: 15,
    response_seconds: 180,
    total_seconds: 240,
};

/// Token 预算。默认 32,768 / 8,192；约束 reserve <= context/2。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub context_window_tokens: u32,
    pub output_reserve_tokens: u32,
}

pub const DEFAULT_BUDGET: Budget = Budget {
    context_window_tokens: 32_768,
    output_reserve_tokens: 8_192,
};

/// 周期策略（cadence/effective_from 必填；默认周一/一月/Asia/Shanghai 由适配层填充）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeriodPolicy {
    pub cadence: Cadence,
    pub week_start_day: u8,
    pub fiscal_year_start_month: u8,
    pub timezone_id: String,
    pub effective_from: NaiveDate,
}

/// CLI/UI 输入（字段可缺省，由核心填充默认值后校验并回显实际采用值）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PeriodPolicyInputDefaults {
    pub cadence: Option<Cadence>,
    pub week_start_day: Option<u8>,
    pub fiscal_year_start_month: Option<u8>,
    pub timezone_id: Option<String>,
    pub effective_from: Option<NaiveDate>,
}

impl PeriodPolicyInputDefaults {
    /// 仅补本文声明的默认值；返回实际采用值。
    pub fn materialize(self) -> Result<PeriodPolicy, &'static str> {
        let cadence = self.cadence.ok_or("cadence_required")?;
        Ok(PeriodPolicy {
            cadence,
            week_start_day: self.week_start_day.unwrap_or(1),
            fiscal_year_start_month: self.fiscal_year_start_month.unwrap_or(1),
            timezone_id: self.timezone_id.unwrap_or_else(|| "Asia/Shanghai".into()),
            effective_from: self.effective_from.ok_or("effective_from_required")?,
        })
    }
}

/// Provider 公开配置（Key 不在此对象中）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPublic {
    pub name: String,
    pub protocol: ChatCompletionsMarker,
    pub base_url: String,
    pub model: String,
    pub stream: bool,
    pub timeouts: Timeouts,
    pub budget: Budget,
}

/// protocol 固定 chat_completions（settings.schema.json const）。
///
/// serde const 校验：仅接受字符串 `"chat_completions"`（其他值拒绝），
/// 序列化恒输出 `"chat_completions"`（unit 形态不出现在 JSON 边界）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatCompletionsMarker;

impl Default for ChatCompletionsMarker {
    fn default() -> Self {
        Self
    }
}

impl Serialize for ChatCompletionsMarker {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("chat_completions")
    }
}

impl<'de> Deserialize<'de> for ChatCompletionsMarker {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = ChatCompletionsMarker;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, r#"const 字符串 "chat_completions""#)
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if v == "chat_completions" {
                    Ok(ChatCompletionsMarker)
                } else {
                    Err(E::custom(
                        r#"protocol 必须为 "chat_completions"（settings.schema const）"#,
                    ))
                }
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

/// 组合约束（写入前校验，失败无部分保存）。
pub fn validate_budget_timeouts(budget: &Budget, timeouts: &Timeouts) -> Result<(), &'static str> {
    if !(8_192..=1_048_576).contains(&budget.context_window_tokens) {
        return Err("context_window_tokens_range");
    }
    if !(1_024..=65_536).contains(&budget.output_reserve_tokens) {
        return Err("output_reserve_tokens_range");
    }
    if budget.output_reserve_tokens * 2 > budget.context_window_tokens {
        return Err("reserve_over_half_context");
    }
    if !(1..=60).contains(&timeouts.connect_seconds) {
        return Err("connect_seconds_range");
    }
    if !(30..=600).contains(&timeouts.response_seconds) {
        return Err("response_seconds_range");
    }
    // 2026-11-04 增量：流式生成总时长上限扩至 1800s
    if !(60..=1800).contains(&timeouts.total_seconds) {
        return Err("total_seconds_range");
    }
    if timeouts.total_seconds < timeouts.connect_seconds + timeouts.response_seconds {
        return Err("total_lt_connect_plus_response");
    }
    Ok(())
}

/// base_url 校验：仅 HTTPS；HTTP 仅限字面 localhost/127.0.0.1/[::1]；
/// 禁 userinfo/query/fragment；拒绝重定向由 HTTP 层执行。
pub fn validate_base_url(base_url: &str) -> Result<(), &'static str> {
    let url = base_url.trim();
    let lower = url.to_ascii_lowercase();
    let (scheme, rest) = lower.split_once("://").ok_or("url_scheme_required")?;
    match scheme {
        "https" => {}
        "http" => {
            let host = host_of(rest).ok_or("url_host_required")?;
            let loopback = host == "localhost" || host == "127.0.0.1" || host == "[::1]";
            if !loopback {
                return Err("http_only_loopback");
            }
        }
        _ => return Err("url_scheme_https_or_loopback_http"),
    }
    if url.contains(['@', '?', '#']) {
        return Err("url_userinfo_query_fragment_forbidden");
    }
    if rest.is_empty() {
        return Err("url_host_required");
    }
    Ok(())
}

/// 取 authority 的 host 部分：先去路径；[IPv6] 括号形式整体返回（端口在括号后），
/// 其余去 :port。
fn host_of(authority_and_path: &str) -> Option<&str> {
    let authority = authority_and_path.split('/').next()?;
    if let Some(stripped) = authority.strip_prefix('[') {
        let end = stripped.find(']')?;
        return Some(&authority[..end + 2]);
    }
    authority.split(':').next()
}

/// 拼接 endpoint：去末尾斜杠后追加 /chat/completions；已含完整路径时提示填写 Base URL。
pub fn chat_completions_url(base_url: &str) -> Result<String, &'static str> {
    validate_base_url(base_url)?;
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.to_ascii_lowercase().ends_with("/chat/completions") {
        return Err("base_url_already_full_path");
    }
    Ok(format!("{trimmed}/chat/completions"))
}

/// 设置操作（settings.schema.json oneOf）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum SettingsOperation {
    WorkspaceCreate {
        name: String,
        initial_period_policy: PeriodPolicy,
    },
    WorkspaceUpdate {
        workspace_id: uuid::Uuid,
        expected_revision: u32,
        patch: WorkspaceNamePatch,
    },
    WorkspaceUse {
        workspace_id: uuid::Uuid,
    },
    PeriodPolicySet {
        workspace_id: uuid::Uuid,
        expected_revision: u32,
        policy: PeriodPolicy,
    },
    ProviderUpsert {
        workspace_id: uuid::Uuid,
        provider_id: Option<uuid::Uuid>,
        expected_revision: Option<u32>,
        config: ProviderPublic,
        key_action: KeyAction,
    },
    ProviderUse {
        workspace_id: uuid::Uuid,
        /// null 表示清除默认。
        provider_id: Option<uuid::Uuid>,
        /// workspace 配置 revision。
        expected_revision: u32,
    },
    ProviderClearKey {
        workspace_id: uuid::Uuid,
        provider_id: uuid::Uuid,
        expected_revision: u32,
        confirm_local: bool,
    },
    ProviderDelete {
        workspace_id: uuid::Uuid,
        provider_id: uuid::Uuid,
        expected_revision: u32,
        confirm_local: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    Keep,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceNamePatch {
    pub name: String,
}

/// 公开视图（settings.schema.json $defs）。

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceView {
    pub workspace_id: uuid::Uuid,
    pub name: String,
    pub config_revision: u32,
    pub active_provider_id: Option<uuid::Uuid>,
    pub current_period_policy_id: uuid::Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderView {
    pub provider_id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub revision: u32,
    pub config: ProviderPublic,
    pub key_present: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeriodPolicyView {
    pub period_policy_id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub version: u32,
    pub policy: PeriodPolicy,
    pub effective_to: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn budget_timeout_constraints() {
        assert!(validate_budget_timeouts(&DEFAULT_BUDGET, &DEFAULT_TIMEOUTS).is_ok());
        let bad_reserve = Budget {
            context_window_tokens: 8_192,
            output_reserve_tokens: 8_192,
        };
        assert!(validate_budget_timeouts(&bad_reserve, &DEFAULT_TIMEOUTS).is_err());
        let bad_total = Timeouts {
            connect_seconds: 60,
            response_seconds: 600,
            total_seconds: 240,
        };
        assert!(validate_budget_timeouts(&DEFAULT_BUDGET, &bad_total).is_err());
    }

    #[test]
    fn url_rules() {
        assert!(validate_base_url("https://api.example.com/v1").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_base_url("http://insecure.example.com").is_err());
        assert!(validate_base_url("https://u:p@example.com").is_err());
        assert!(validate_base_url("https://example.com/v1?x=1").is_err());
        assert_eq!(
            chat_completions_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com/v1/chat/completions"
        );
        assert!(chat_completions_url("https://api.example.com/v1/chat/completions").is_err());
    }

    #[test]
    fn policy_defaults() {
        let input = PeriodPolicyInputDefaults {
            cadence: Some(Cadence::Month),
            effective_from: Some(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()),
            ..Default::default()
        };
        let p = input.materialize().unwrap();
        assert_eq!(p.week_start_day, 1);
        assert_eq!(p.timezone_id, "Asia/Shanghai");
        assert!(PeriodPolicyInputDefaults::default().materialize().is_err());
    }

    #[test]
    fn chat_completions_marker_serde_const() {
        // 反序列化：仅 const 字符串
        assert!(serde_json::from_value::<ChatCompletionsMarker>(json!("chat_completions")).is_ok());
        for bad in [
            json!("openai"),
            json!(""),
            json!(null),
            json!(1),
            json!(true),
        ] {
            assert!(
                serde_json::from_value::<ChatCompletionsMarker>(bad).is_err(),
                "非 const 值必须拒绝"
            );
        }
        // 序列化：恒输出 const（unit 形态不出现在 JSON 边界）
        assert_eq!(
            serde_json::to_value(ChatCompletionsMarker).unwrap(),
            json!("chat_completions")
        );
        // ProviderPublic 往返（settings.schema protocol const）
        let config = json!({
            "name": "p", "protocol": "chat_completions",
            "base_url": "https://api.example.com/v1", "model": "m", "stream": false,
            "timeouts": DEFAULT_TIMEOUTS, "budget": DEFAULT_BUDGET
        });
        let parsed: ProviderPublic = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.protocol, ChatCompletionsMarker);
        assert_eq!(
            serde_json::to_value(&parsed).unwrap()["protocol"],
            json!("chat_completions")
        );
        // 非 const 在 ProviderPublic 边界即被拒
        let bad = json!({
            "name": "p", "protocol": "openai",
            "base_url": "https://api.example.com/v1", "model": "m", "stream": false,
            "timeouts": DEFAULT_TIMEOUTS, "budget": DEFAULT_BUDGET
        });
        let err = serde_json::from_value::<ProviderPublic>(bad).unwrap_err();
        assert!(err.to_string().contains("chat_completions"), "{err}");
    }
}
