//! The agent orchestration loop. Mirrors `gauss/core/agent/agent.py`
//! (`Agent.send_message`) — resolve user, optional workflow short-circuit,
//! build prompt, run the LLM↔tool loop, and stream UI components throughout.

use crate::components::{RichComponent, SimpleComponent, Task, TaskOperation, UiComponent};
use crate::context::ToolContext;
use crate::model::agent_config::{ui_feature, AgentConfig};
use crate::model::conversation::{Conversation, Message};
use crate::model::llm::{LlmMessage, LlmRequest, LlmResponse};
use crate::model::tool::ToolSchema;
use crate::model::user::{RequestContext, User};
use crate::tool::ToolRegistry;
use crate::traits::{
    AgentMemory, AuditLogger, ConversationFilter, ConversationStore, LifecycleHook,
    LlmContextEnhancer, LlmMiddleware, LlmService, ObservabilityProvider, SystemPromptBuilder,
    ToolContextEnricher,
};
use crate::workflow::{DefaultWorkflowHandler, WorkflowHandler};
use futures::Stream;
use futures::StreamExt;
use std::sync::Arc;
use uuid::Uuid;

/// The agent. Construct via [`AgentBuilder`].
pub struct Agent {
    llm_service: Arc<dyn LlmService>,
    tool_registry: ToolRegistry,
    user_resolver: Arc<dyn crate::traits::UserResolver>,
    agent_memory: Arc<dyn AgentMemory>,
    conversation_store: Arc<dyn ConversationStore>,
    config: AgentConfig,
    system_prompt_builder: Arc<dyn SystemPromptBuilder>,
    workflow_handler: Option<Arc<dyn WorkflowHandler>>,
    llm_context_enhancer: Option<Arc<dyn LlmContextEnhancer>>,
    lifecycle_hooks: Vec<Arc<dyn LifecycleHook>>,
    llm_middlewares: Vec<Arc<dyn LlmMiddleware>>,
    conversation_filters: Vec<Arc<dyn ConversationFilter>>,
    context_enrichers: Vec<Arc<dyn ToolContextEnricher>>,
    observability_provider: Option<Arc<dyn ObservabilityProvider>>,
    audit_logger: Option<Arc<dyn AuditLogger>>,
    error_recovery_strategy: Option<Arc<dyn crate::recovery::ErrorRecoveryStrategy>>,
}

impl Agent {
    pub fn tool_registry(&self) -> &ToolRegistry {
        &self.tool_registry
    }

    pub fn agent_memory(&self) -> &Arc<dyn AgentMemory> {
        &self.agent_memory
    }

    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// Process a user message, yielding UI components as a stream.
    pub fn send_message(
        self: Arc<Self>,
        request_context: RequestContext,
        message: String,
        conversation_id: Option<String>,
    ) -> impl Stream<Item = UiComponent> {
        async_stream::stream! {
            // 1. Resolve user.
            let user = match self.user_resolver.resolve_user(&request_context).await {
                Ok(u) => u,
                Err(e) => {
                    yield error_card(&format!("Could not resolve user: {e}"));
                    return;
                }
            };

            let conversation_id =
                conversation_id.unwrap_or_else(|| Uuid::new_v4().to_string());

            // 2. Starter-UI request (empty message or explicit flag).
            let is_starter = message.trim().is_empty()
                || request_context
                    .metadata
                    .get("starter_ui_request")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);

            if is_starter {
                if let Some(wf) = &self.workflow_handler {
                    let conversation = self.load_or_new(&conversation_id, &user).await;
                    if let Some(components) =
                        wf.get_starter_ui(&self, &user, &conversation).await
                    {
                        for c in components {
                            yield c;
                        }
                        yield UiComponent::new(RichComponent::status_bar(
                            "idle", "Ready", Some("Choose an option or type a message".into()),
                        ));
                        yield UiComponent::new(RichComponent::chat_input(
                            "Ask a question...", false,
                        ));
                    }
                }
                return;
            }

            // 3. before_message lifecycle hooks.
            let mut message = message;
            for hook in &self.lifecycle_hooks {
                if let Some(m) = hook.before_message(&user, &message).await {
                    message = m;
                }
            }

            yield UiComponent::new(RichComponent::status_bar(
                "working", "Processing your request...", Some("Analyzing query".into()),
            ));

            // 4. Load or create the conversation.
            let mut conversation = self.load_or_new(&conversation_id, &user).await;

            // 5. Workflow short-circuit.
            if let Some(wf) = &self.workflow_handler {
                let result = wf.try_handle(&self, &user, &conversation, &message).await;
                if result.should_skip_llm {
                    for c in result.components {
                        yield c;
                    }
                    yield UiComponent::new(RichComponent::status_bar(
                        "idle", "Workflow complete", Some("Ready for next message".into()),
                    ));
                    yield UiComponent::new(RichComponent::chat_input(
                        "Ask a question...", false,
                    ));
                    if self.config.auto_save_conversations {
                        let _ = self.conversation_store.update_conversation(&conversation).await;
                    }
                    return;
                }
            }

            // 6. Append the user message.
            conversation.add_message(Message::user(&message));

            // 7. Build the tool context, then run enrichers over it.
            let request_id = Uuid::new_v4().to_string();
            let mut context = ToolContext::new(
                user.clone(),
                conversation_id.clone(),
                request_id.clone(),
                self.agent_memory.clone(),
            );
            context.observability_provider = self.observability_provider.clone();
            for enricher in &self.context_enrichers {
                context = enricher.enrich_context(context).await;
            }

            // Observability + audit: message received.
            if let Some(obs) = &self.observability_provider {
                obs.record_metric("agent.message", 1.0, "count", None).await;
            }
            if let Some(audit) = &self.audit_logger {
                if self.config.audit_config.enabled {
                    let _ = audit
                        .log_event(crate::model::audit::AuditEvent::new(
                            crate::model::audit::AuditEventType::MessageReceived,
                            &user,
                            &conversation_id,
                            &request_id,
                        ))
                        .await;
                }
            }

            // 8. Tool schemas visible to this user.
            let schemas = self.tool_registry.get_schemas(&user);

            // 9. System prompt (+ optional RAG enhancement).
            let mut system_prompt = self
                .system_prompt_builder
                .build_system_prompt(&user, &schemas)
                .await
                .ok()
                .flatten();
            if let (Some(enh), Some(sp)) = (&self.llm_context_enhancer, system_prompt.clone()) {
                system_prompt = Some(enh.enhance_system_prompt(sp, &message, &user).await);
            }

            // 10. The tool loop.
            let mut request = self
                .build_llm_request(&conversation, &schemas, &user, system_prompt.clone())
                .await;
            let mut iterations: u32 = 0;
            let show_tool_names =
                self.config.ui_features.can_user_access_feature(ui_feature::SHOW_TOOL_NAMES, &user);
            let show_tool_args =
                self.config.ui_features.can_user_access_feature(ui_feature::SHOW_TOOL_ARGUMENTS, &user);
            let show_tool_error =
                self.config.ui_features.can_user_access_feature(ui_feature::SHOW_TOOL_ERROR, &user);

            loop {
                if iterations >= self.config.max_tool_iterations {
                    yield UiComponent::new(RichComponent::status_bar(
                        "warning",
                        "Tool limit reached",
                        Some(format!("Stopped after {iterations} tool executions.")),
                    ));
                    yield UiComponent::with_simple(
                        RichComponent::text(
                            format!(
                                "⚠️ Tool execution limit reached after {iterations} tools. \
                                 The task may be incomplete."
                            ),
                            true,
                        ),
                        SimpleComponent::text("Tool limit reached; task may be incomplete."),
                    );
                    yield UiComponent::new(RichComponent::chat_input(
                        "Continue the task or ask something else...", false,
                    ));
                    break;
                }

                let response = self.get_llm_response(request).await;

                if response.is_tool_call() {
                    iterations += 1;
                    let tool_calls = response.tool_calls.clone().unwrap_or_default();

                    conversation.add_message(
                        Message::assistant(response.content.clone().unwrap_or_default())
                            .with_tool_calls(tool_calls.clone()),
                    );

                    yield UiComponent::new(RichComponent::status_bar(
                        "working",
                        "Executing tools...",
                        Some(format!("Running {} tool(s)", tool_calls.len())),
                    ));

                    let mut tool_responses: Vec<(String, String)> = Vec::new();

                    for tool_call in &tool_calls {
                        if show_tool_names {
                            let task = Task::new(
                                format!("Execute {}", tool_call.name),
                                Some("Running tool with provided arguments".into()),
                                "in_progress",
                            );
                            yield UiComponent::new(RichComponent::task_tracker(
                                TaskOperation::AddTask, Some(task), None, None, None,
                            ));
                        }
                        if show_tool_args {
                            yield UiComponent::new(RichComponent::status_card(
                                format!("Executing {}", tool_call.name),
                                "running",
                                Some(format!("{} argument(s)", tool_call.arguments.len())),
                                Some("⚙️".into()),
                                tool_call.arguments.clone(),
                            ));
                        }

                        // before_tool hooks.
                        for hook in &self.lifecycle_hooks {
                            let _ = hook.before_tool(&tool_call.name, &context).await;
                        }

                        // Observability span around the tool execution.
                        let mut tool_span = match &self.observability_provider {
                            Some(obs) => Some(obs.create_span("agent.tool.execute").await),
                            None => None,
                        };

                        let mut result =
                            self.tool_registry.execute(tool_call, &context).await;

                        if let Some(obs) = &self.observability_provider {
                            obs.record_metric("agent.tool.execute", 1.0, "count", None).await;
                            if let Some(span) = tool_span.as_mut() {
                                obs.end_span(span).await;
                            }
                        }

                        // after_tool hooks (None → leave result unchanged).
                        for hook in &self.lifecycle_hooks {
                            if let Some(modified) = hook.after_tool(result.clone()).await {
                                result = modified;
                            }
                        }

                        // Stream the tool's UI component (errors gated by feature).
                        if let Some(component) = result.ui_component.take() {
                            if result.success || show_tool_error {
                                yield component;
                            }
                        }

                        tool_responses.push((
                            tool_call.id.clone(),
                            if result.success {
                                result.result_for_llm.clone()
                            } else {
                                result.error.clone().unwrap_or_else(|| "Tool execution failed".into())
                            },
                        ));
                    }

                    for (id, content) in tool_responses {
                        conversation.add_message(Message::tool_response(id, content));
                    }

                    request = self
                        .build_llm_request(&conversation, &schemas, &user, system_prompt.clone())
                        .await;
                } else {
                    yield UiComponent::new(RichComponent::status_bar(
                        "idle", "Response complete", Some("Ready for next message".into()),
                    ));
                    yield UiComponent::new(RichComponent::chat_input(
                        "Ask a follow-up question...", false,
                    ));
                    if let Some(content) = response.content {
                        // Audit: AI response generated.
                        if let Some(audit) = &self.audit_logger {
                            if self.config.audit_config.enabled
                                && self.config.audit_config.log_ai_responses
                            {
                                audit
                                    .log_ai_response(
                                        &user,
                                        &conversation_id,
                                        &request_id,
                                        &content,
                                        0,
                                        self.config.audit_config.include_full_ai_responses,
                                    )
                                    .await;
                            }
                        }
                        conversation.add_message(Message::assistant(&content));
                        yield UiComponent::with_simple(
                            RichComponent::text(content.clone(), true),
                            SimpleComponent::text(content),
                        );
                    }
                    break;
                }
            }

            // 11. Persist + after_message hooks.
            if self.config.auto_save_conversations {
                let _ = self.conversation_store.update_conversation(&conversation).await;
            }
            for hook in &self.lifecycle_hooks {
                hook.after_message(&conversation).await;
            }
        }
    }

    async fn load_or_new(&self, conversation_id: &str, user: &User) -> Conversation {
        match self
            .conversation_store
            .get_conversation(conversation_id, user)
            .await
        {
            Ok(Some(c)) => c,
            _ => Conversation::new(conversation_id, user.clone()),
        }
    }

    async fn build_llm_request(
        &self,
        conversation: &Conversation,
        schemas: &[ToolSchema],
        user: &User,
        system_prompt: Option<String>,
    ) -> LlmRequest {
        let mut messages = conversation.messages.clone();
        for filter in &self.conversation_filters {
            messages = filter.filter_messages(messages).await;
        }
        let mut llm_messages: Vec<LlmMessage> = messages
            .into_iter()
            .map(|m| LlmMessage {
                role: m.role,
                content: m.content,
                tool_calls: m.tool_calls,
                tool_call_id: m.tool_call_id,
            })
            .collect();
        if let Some(enh) = &self.llm_context_enhancer {
            llm_messages = enh.enhance_user_messages(llm_messages, user).await;
        }

        LlmRequest {
            messages: llm_messages,
            tools: if schemas.is_empty() {
                None
            } else {
                Some(schemas.to_vec())
            },
            user: user.clone(),
            stream: self.config.stream_responses,
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
            system_prompt,
            metadata: Default::default(),
        }
    }

    /// A single LLM attempt (streaming-accumulated or one-shot), returning an
    /// error so the recovery loop can decide what to do.
    async fn invoke_llm_once(&self, request: &LlmRequest) -> crate::Result<LlmResponse> {
        if self.config.stream_responses {
            let mut content = String::new();
            let mut tool_calls = Vec::new();
            let mut stream = self.llm_service.stream_request(request.clone()).await;
            while let Some(chunk) = stream.next().await {
                let c = chunk?;
                if let Some(t) = c.content {
                    content.push_str(&t);
                }
                if let Some(tc) = c.tool_calls {
                    tool_calls.extend(tc);
                }
            }
            Ok(LlmResponse {
                content: (!content.is_empty()).then_some(content),
                tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
                ..Default::default()
            })
        } else {
            self.llm_service.send_request(request.clone()).await
        }
    }

    async fn get_llm_response(&self, mut request: LlmRequest) -> LlmResponse {
        use crate::recovery::{RecoveryAction, RecoveryActionType};
        // Hard cap regardless of strategy, to guarantee termination.
        const HARD_CAP: u32 = 5;

        for mw in &self.llm_middlewares {
            request = mw.before_llm_request(request).await;
        }

        let mut attempt = 1u32;
        let mut response = loop {
            match self.invoke_llm_once(&request).await {
                Ok(r) => break r,
                Err(e) => {
                    let action = match &self.error_recovery_strategy {
                        Some(s) => s.handle_llm_error(&e.to_string(), &request, attempt).await,
                        None => RecoveryAction::fail(e.to_string()),
                    };
                    match action.action {
                        RecoveryActionType::Retry if attempt < HARD_CAP => {
                            if action.retry_delay_ms > 0 {
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    action.retry_delay_ms,
                                ))
                                .await;
                            }
                            attempt += 1;
                            continue;
                        }
                        RecoveryActionType::Fallback => {
                            break LlmResponse {
                                content: Some(
                                    action
                                        .fallback_value
                                        .or(action.message)
                                        .unwrap_or_else(|| format!("LLM error: {e}")),
                                ),
                                ..Default::default()
                            };
                        }
                        _ => {
                            break LlmResponse {
                                content: Some(
                                    action.message.unwrap_or_else(|| format!("LLM error: {e}")),
                                ),
                                ..Default::default()
                            };
                        }
                    }
                }
            }
        };

        for mw in &self.llm_middlewares {
            response = mw.after_llm_response(&request, response).await;
        }
        response
    }
}

fn error_card(message: &str) -> UiComponent {
    UiComponent::with_simple(
        RichComponent::status_card(
            "Error Processing Message",
            "error",
            Some(message.to_string()),
            Some("⚠️".into()),
            Default::default(),
        ),
        SimpleComponent::text(format!("Error: {message}")),
    )
}

/// Builder for [`Agent`]. Required: LLM service, tool registry, user resolver,
/// agent memory. Everything else has sensible defaults.
pub struct AgentBuilder {
    llm_service: Arc<dyn LlmService>,
    tool_registry: ToolRegistry,
    user_resolver: Arc<dyn crate::traits::UserResolver>,
    agent_memory: Arc<dyn AgentMemory>,
    conversation_store: Option<Arc<dyn ConversationStore>>,
    config: AgentConfig,
    system_prompt_builder: Option<Arc<dyn SystemPromptBuilder>>,
    workflow_handler: Option<Arc<dyn WorkflowHandler>>,
    llm_context_enhancer: Option<Arc<dyn LlmContextEnhancer>>,
    lifecycle_hooks: Vec<Arc<dyn LifecycleHook>>,
    llm_middlewares: Vec<Arc<dyn LlmMiddleware>>,
    conversation_filters: Vec<Arc<dyn ConversationFilter>>,
    context_enrichers: Vec<Arc<dyn ToolContextEnricher>>,
    observability_provider: Option<Arc<dyn ObservabilityProvider>>,
    audit_logger: Option<Arc<dyn AuditLogger>>,
    error_recovery_strategy: Option<Arc<dyn crate::recovery::ErrorRecoveryStrategy>>,
}

impl AgentBuilder {
    pub fn new(
        llm_service: Arc<dyn LlmService>,
        tool_registry: ToolRegistry,
        user_resolver: Arc<dyn crate::traits::UserResolver>,
        agent_memory: Arc<dyn AgentMemory>,
    ) -> Self {
        Self {
            llm_service,
            tool_registry,
            user_resolver,
            agent_memory,
            conversation_store: None,
            config: AgentConfig::default(),
            system_prompt_builder: None,
            workflow_handler: None,
            llm_context_enhancer: None,
            lifecycle_hooks: Vec::new(),
            llm_middlewares: Vec::new(),
            conversation_filters: Vec::new(),
            context_enrichers: Vec::new(),
            observability_provider: None,
            audit_logger: None,
            error_recovery_strategy: None,
        }
    }

    pub fn config(mut self, config: AgentConfig) -> Self {
        self.config = config;
        self
    }

    pub fn conversation_store(mut self, store: Arc<dyn ConversationStore>) -> Self {
        self.conversation_store = Some(store);
        self
    }

    pub fn system_prompt_builder(mut self, builder: Arc<dyn SystemPromptBuilder>) -> Self {
        self.system_prompt_builder = Some(builder);
        self
    }

    pub fn workflow_handler(mut self, handler: Arc<dyn WorkflowHandler>) -> Self {
        self.workflow_handler = Some(handler);
        self
    }

    pub fn llm_context_enhancer(mut self, enhancer: Arc<dyn LlmContextEnhancer>) -> Self {
        self.llm_context_enhancer = Some(enhancer);
        self
    }

    pub fn add_lifecycle_hook(mut self, hook: Arc<dyn LifecycleHook>) -> Self {
        self.lifecycle_hooks.push(hook);
        self
    }

    pub fn add_llm_middleware(mut self, mw: Arc<dyn LlmMiddleware>) -> Self {
        self.llm_middlewares.push(mw);
        self
    }

    pub fn add_conversation_filter(mut self, filter: Arc<dyn ConversationFilter>) -> Self {
        self.conversation_filters.push(filter);
        self
    }

    pub fn observability_provider(mut self, provider: Arc<dyn ObservabilityProvider>) -> Self {
        self.observability_provider = Some(provider);
        self
    }

    pub fn add_context_enricher(mut self, enricher: Arc<dyn ToolContextEnricher>) -> Self {
        self.context_enrichers.push(enricher);
        self
    }

    pub fn audit_logger(mut self, logger: Arc<dyn AuditLogger>) -> Self {
        self.audit_logger = Some(logger);
        self
    }

    pub fn error_recovery_strategy(
        mut self,
        strategy: Arc<dyn crate::recovery::ErrorRecoveryStrategy>,
    ) -> Self {
        self.error_recovery_strategy = Some(strategy);
        self
    }

    pub fn build(self) -> Agent {
        // Wire the audit logger into the tool registry so tool events are logged.
        let mut tool_registry = self.tool_registry;
        if let Some(audit) = &self.audit_logger {
            if self.config.audit_config.enabled {
                tool_registry.set_audit(audit.clone(), self.config.audit_config.clone());
            }
        }

        Agent {
            llm_service: self.llm_service,
            tool_registry,
            user_resolver: self.user_resolver,
            agent_memory: self.agent_memory,
            conversation_store: self
                .conversation_store
                .unwrap_or_else(|| Arc::new(crate::defaults::InMemoryConversationStore::new())),
            config: self.config,
            system_prompt_builder: self
                .system_prompt_builder
                .unwrap_or_else(|| Arc::new(crate::prompt::DefaultSystemPromptBuilder::new())),
            workflow_handler: Some(
                self.workflow_handler
                    .unwrap_or_else(|| Arc::new(DefaultWorkflowHandler)),
            ),
            llm_context_enhancer: self.llm_context_enhancer,
            lifecycle_hooks: self.lifecycle_hooks,
            llm_middlewares: self.llm_middlewares,
            conversation_filters: self.conversation_filters,
            context_enrichers: self.context_enrichers,
            observability_provider: self.observability_provider,
            audit_logger: self.audit_logger,
            error_recovery_strategy: self.error_recovery_strategy,
        }
    }
}
