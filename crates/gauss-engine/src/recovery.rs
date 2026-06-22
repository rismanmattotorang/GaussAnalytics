//! Error-recovery strategy. Mirrors `gauss/core/recovery/{base,models}.py`.
//!
//! When an LLM (or tool) call fails, the agent consults an
//! [`ErrorRecoveryStrategy`] to decide whether to retry, fail, fall back to a
//! canned value, or skip.

use crate::model::llm::LlmRequest;
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryActionType {
    Retry,
    Fail,
    Fallback,
    Skip,
}

#[derive(Debug, Clone)]
pub struct RecoveryAction {
    pub action: RecoveryActionType,
    pub retry_delay_ms: u64,
    pub fallback_value: Option<String>,
    pub message: Option<String>,
}

impl RecoveryAction {
    pub fn fail(message: impl Into<String>) -> Self {
        Self {
            action: RecoveryActionType::Fail,
            retry_delay_ms: 0,
            fallback_value: None,
            message: Some(message.into()),
        }
    }

    pub fn retry(retry_delay_ms: u64) -> Self {
        Self {
            action: RecoveryActionType::Retry,
            retry_delay_ms,
            fallback_value: None,
            message: None,
        }
    }

    pub fn fallback(value: impl Into<String>) -> Self {
        Self {
            action: RecoveryActionType::Fallback,
            retry_delay_ms: 0,
            fallback_value: Some(value.into()),
            message: None,
        }
    }

    pub fn skip() -> Self {
        Self {
            action: RecoveryActionType::Skip,
            retry_delay_ms: 0,
            fallback_value: None,
            message: None,
        }
    }
}

/// Decides how to recover from LLM/tool errors. All methods default to failing.
#[async_trait]
pub trait ErrorRecoveryStrategy: Send + Sync {
    async fn handle_llm_error(
        &self,
        error: &str,
        _request: &LlmRequest,
        _attempt: u32,
    ) -> RecoveryAction {
        RecoveryAction::fail(error.to_string())
    }

    async fn handle_tool_error(&self, error: &str, _attempt: u32) -> RecoveryAction {
        RecoveryAction::fail(error.to_string())
    }
}

/// Retries up to `max_attempts` times with a fixed delay, then fails.
pub struct RetryStrategy {
    pub max_attempts: u32,
    pub delay_ms: u64,
}

impl RetryStrategy {
    pub fn new(max_attempts: u32, delay_ms: u64) -> Self {
        Self {
            max_attempts,
            delay_ms,
        }
    }
}

#[async_trait]
impl ErrorRecoveryStrategy for RetryStrategy {
    async fn handle_llm_error(
        &self,
        error: &str,
        _request: &LlmRequest,
        attempt: u32,
    ) -> RecoveryAction {
        if attempt < self.max_attempts {
            RecoveryAction::retry(self.delay_ms)
        } else {
            RecoveryAction::fail(error.to_string())
        }
    }

    async fn handle_tool_error(&self, error: &str, attempt: u32) -> RecoveryAction {
        if attempt < self.max_attempts {
            RecoveryAction::retry(self.delay_ms)
        } else {
            RecoveryAction::fail(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::user::User;

    fn req() -> LlmRequest {
        LlmRequest {
            messages: vec![],
            tools: None,
            user: User::new("u"),
            stream: false,
            temperature: 0.0,
            max_tokens: None,
            system_prompt: None,
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn retry_then_fail() {
        let s = RetryStrategy::new(3, 0);
        assert_eq!(
            s.handle_llm_error("boom", &req(), 1).await.action,
            RecoveryActionType::Retry
        );
        assert_eq!(
            s.handle_llm_error("boom", &req(), 2).await.action,
            RecoveryActionType::Retry
        );
        // attempt == max_attempts → fail.
        assert_eq!(
            s.handle_llm_error("boom", &req(), 3).await.action,
            RecoveryActionType::Fail
        );
    }
}
