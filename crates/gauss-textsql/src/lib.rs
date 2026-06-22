//! Self-correcting text-to-SQL pipeline, exposed as a [`TextToSqlTool`].
//!
//! Implements the SOTA recipe (DIN-SQL / MAC-SQL / DAIL-SQL) over GaussAnalytics's
//! semantic layer:
//! schema linking → few-shot retrieval → generation → AST guardrails (dry-plan)
//! → execution + execution-guided self-correction (dry-run) → self-learning.

pub use gauss_sqlguard::Guardrails;

use async_trait::async_trait;
use gauss_engine::components::{RichComponent, UiComponent};
use gauss_engine::context::{ToolContext, ToolResult};
use gauss_engine::model::llm::{LlmMessage, LlmRequest};
use gauss_engine::tool::Tool;
use gauss_engine::traits::{LlmService, SqlRunner};
use gauss_semantic::SemanticModel;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct TextToSqlConfig {
    /// Extra generations allowed after the first, each fed the prior error.
    pub max_correction_attempts: u32,
    /// Number of independent candidates per step (reserved; see roadmap).
    pub candidates: u32,
    pub enforce_read_only: bool,
    /// Injected LIMIT for limit-less queries (`None` disables).
    pub default_limit: Option<u64>,
    /// Use an LLM schema-linking step when the model has more than this many models.
    pub schema_linking_threshold: usize,
    pub few_shot_limit: usize,
    /// Mask values in PII-flagged columns for users lacking PII access.
    pub redact_pii: bool,
    /// Groups whose members may see PII columns unmasked.
    pub pii_access_groups: Vec<String>,
    /// Ask clarifying questions when the question is ambiguous, instead of
    /// guessing at SQL. Off by default (single-shot answering).
    pub clarify: bool,
    /// Ground filters in real column values (only acts on semantic columns
    /// flagged `link_values`). On by default.
    pub value_linking: bool,
    /// Max distinct values fetched per value-linked column.
    pub value_sample_limit: usize,
}

impl Default for TextToSqlConfig {
    fn default() -> Self {
        Self {
            max_correction_attempts: 2,
            candidates: 1,
            enforce_read_only: true,
            default_limit: Some(1000),
            schema_linking_threshold: 8,
            few_shot_limit: 5,
            redact_pii: true,
            pii_access_groups: vec!["view_pii".into()],
            clarify: false,
            value_linking: true,
            value_sample_limit: 20,
        }
    }
}

/// A self-contained text-to-SQL tool: it owns an LLM, a SQL runner, and the
/// semantic model, and runs the full generate→validate→repair pipeline.
pub struct TextToSqlTool {
    llm: Arc<dyn LlmService>,
    runner: Arc<dyn SqlRunner>,
    semantic: Arc<SemanticModel>,
    config: TextToSqlConfig,
    access_groups: Vec<String>,
    /// Cache of fetched distinct column values, keyed by `table.column`.
    value_hint_cache: std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>,
}

impl TextToSqlTool {
    pub fn new(
        llm: Arc<dyn LlmService>,
        runner: Arc<dyn SqlRunner>,
        semantic: Arc<SemanticModel>,
    ) -> Self {
        Self {
            llm,
            runner,
            semantic,
            config: TextToSqlConfig::default(),
            access_groups: vec!["user".into(), "admin".into()],
            value_hint_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn with_config(mut self, config: TextToSqlConfig) -> Self {
        self.config = config;
        self
    }

    /// Enable the clarification step (ask before guessing on ambiguous questions).
    pub fn with_clarification(mut self, enabled: bool) -> Self {
        self.config.clarify = enabled;
        self
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TextToSqlArgs {
    /// The natural-language question to answer with SQL.
    pub question: String,
}

/// Extract a SQL statement from an LLM response (fenced block or raw).
pub fn extract_sql(text: &str) -> String {
    let t = text.trim();
    // ```sql ... ``` or ``` ... ```
    if let Some(start) = t.find("```") {
        let after = &t[start + 3..];
        let after = after.strip_prefix("sql").unwrap_or(after);
        let after = after.trim_start_matches(['\n', '\r', ' ']);
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    t.to_string()
}

impl TextToSqlTool {
    /// Ask the LLM whether the question is answerable as-is. Returns
    /// `Some(questions)` when clarification is needed, `None` to proceed.
    async fn clarify_check(&self, question: &str, schema_context: &str) -> Option<Vec<String>> {
        let system = format!(
            "You assess whether a natural-language data question can be answered \
             unambiguously with the data model below. If it is clear enough to write \
             SQL, reply with exactly `OK`. If it is ambiguous or missing key details \
             (e.g. time range, which metric, which entity), reply with `CLARIFY:` \
             followed by 1-3 short clarifying questions separated by ` | `.\n\n{schema_context}"
        );
        let req = LlmRequest {
            messages: vec![LlmMessage::new("user", format!("Question: {question}"))],
            tools: None,
            user: gauss_engine::model::user::User::new("nl2sql"),
            stream: false,
            temperature: 0.0,
            max_tokens: None,
            system_prompt: Some(system),
            metadata: Default::default(),
        };
        let content = self.llm.send_request(req).await.ok()?.content?;
        let trimmed = content.trim();
        let rest = trimmed.strip_prefix("CLARIFY:").or_else(|| {
            // Tolerate a fenced/quoted prefix.
            trimmed
                .find("CLARIFY:")
                .map(|i| &trimmed[i + "CLARIFY:".len()..])
        })?;
        let questions: Vec<String> = rest
            .split('|')
            .map(|q| q.trim().trim_end_matches('?').trim().to_string())
            .filter(|q| !q.is_empty())
            .map(|q| format!("{q}?"))
            .collect();
        if questions.is_empty() {
            None
        } else {
            Some(questions)
        }
    }

    async fn link_schema(&self, question: &str, names: &[String]) -> Vec<String> {
        let prompt = format!(
            "Given the question and the list of available data models, return ONLY a \
             comma-separated list of the model names relevant to answering it.\n\n\
             Question: {question}\nModels: {}\n",
            names.join(", ")
        );
        let req = LlmRequest {
            messages: vec![LlmMessage::new("user", prompt)],
            tools: None,
            user: gauss_engine::model::user::User::new("nl2sql"),
            stream: false,
            temperature: 0.0,
            max_tokens: None,
            system_prompt: None,
            metadata: Default::default(),
        };
        let known: HashSet<&str> = names.iter().map(String::as_str).collect();
        match self.llm.send_request(req).await {
            Ok(resp) => {
                let picked: Vec<String> = resp
                    .content
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| known.contains(s.as_str()))
                    .collect();
                if picked.is_empty() {
                    names.to_vec()
                } else {
                    picked
                }
            }
            Err(_) => names.to_vec(),
        }
    }

    async fn generate(
        &self,
        question: &str,
        context: &str,
        examples: &str,
        feedback: Option<&(String, String)>,
        variant: usize,
    ) -> String {
        let system = format!(
            "You are an expert data analyst. Write a single correct, read-only SQL \
             SELECT query that answers the user's question using ONLY the tables and \
             columns in the data model below. Prefer the documented relationships for \
             joins and the metrics for aggregations. Return ONLY the SQL inside a \
             ```sql code block.\n\n{context}"
        );
        let mut user = String::new();
        if !examples.is_empty() {
            user.push_str(&format!("Similar solved questions:\n{examples}\n\n"));
        }
        user.push_str(&format!("Question: {question}\n"));
        if variant > 0 {
            user.push_str(
                "\nProvide an alternative correct query that takes a different approach \
                 than the obvious one.\n",
            );
        }
        if let Some((bad, err)) = feedback {
            user.push_str(&format!(
                "\nYour previous attempt:\n```sql\n{bad}\n```\nfailed with: {err}\n\
                 Write a corrected query that fixes this.\n"
            ));
        }

        let req = LlmRequest {
            messages: vec![LlmMessage::new("user", user)],
            tools: None,
            user: gauss_engine::model::user::User::new("nl2sql"),
            stream: false,
            temperature: 0.0,
            max_tokens: None,
            system_prompt: Some(system),
            metadata: Default::default(),
        };
        match self.llm.send_request(req).await {
            Ok(resp) => resp.content.unwrap_or_default(),
            Err(e) => format!("-- llm error: {e}"),
        }
    }

    /// Build a "known column values" block for the value-linked columns of the
    /// in-scope models, so the LLM filters on real stored values (e.g. `DE`,
    /// not `Germany`). Pre-supplied values are used as-is; otherwise distinct
    /// values are fetched (and cached) from the database.
    async fn value_hints(&self, context: &ToolContext, models: &[String]) -> String {
        if !self.config.value_linking {
            return String::new();
        }
        let mut lines = Vec::new();
        for (table, column, presupplied) in self.semantic.value_linkable_columns(models) {
            let values = if presupplied.is_empty() {
                self.fetch_distinct(context, &table, &column).await
            } else {
                presupplied
            };
            if !values.is_empty() {
                lines.push(format!("- {table}.{column} values: {}", values.join(", ")));
            }
        }
        if lines.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n## Known column values (use these exact values in WHERE filters)\n{}",
                lines.join("\n")
            )
        }
    }

    /// Fetch (and cache) distinct values for a column. The table/column come
    /// from the trusted semantic model, not user input.
    async fn fetch_distinct(
        &self,
        context: &ToolContext,
        table: &str,
        column: &str,
    ) -> Vec<String> {
        let key = format!("{table}.{column}");
        if let Some(cached) = self.value_hint_cache.lock().unwrap().get(&key) {
            return cached.clone();
        }
        let sql = format!(
            "SELECT DISTINCT \"{column}\" FROM \"{table}\" LIMIT {}",
            self.config.value_sample_limit
        );
        let values = match self.runner.run_sql(&sql, context).await {
            Ok(df) => df
                .rows
                .iter()
                .filter_map(|row| row.first())
                .filter_map(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Null => None,
                    other => Some(other.to_string()),
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        self.value_hint_cache
            .lock()
            .unwrap()
            .insert(key, values.clone());
        values
    }

    async fn run_pipeline(&self, context: &ToolContext, question: &str) -> ToolResult {
        // 1. Schema linking.
        let names = self.semantic.model_names();
        let relevant = if names.len() > self.config.schema_linking_threshold {
            self.link_schema(question, &names).await
        } else {
            names.clone()
        };
        let mut schema_context = self.semantic.render_subset(&relevant);

        // 1a. Value linking: ground filters in real column values.
        let hints = self.value_hints(context, &relevant).await;
        if !hints.is_empty() {
            schema_context.push_str(&hints);
        }

        // 1b. Clarification: if the question is ambiguous, ask instead of guessing.
        if self.config.clarify {
            if let Some(questions) = self.clarify_check(question, &schema_context).await {
                let list = questions
                    .iter()
                    .map(|q| format!("- {q}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let body = format!(
                    "I need a bit more detail before I can answer that accurately:\n\n{list}"
                );
                return ToolResult::success(format!(
                    "The question is ambiguous; ask the user these clarifying questions \
                     before writing SQL:\n{list}"
                ))
                .with_ui(UiComponent::new(RichComponent::card(
                    "Clarification needed",
                    body,
                    true,
                )));
            }
        }

        // 2. Few-shot retrieval from memory.
        let examples = match context
            .agent_memory
            .search_similar_usage(question, context, self.config.few_shot_limit, 0.3, None)
            .await
        {
            Ok(results) => results
                .iter()
                .filter_map(|r| {
                    r.memory
                        .args
                        .get("sql")
                        .and_then(|v| v.as_str())
                        .map(|sql| format!("Q: {}\nSQL: {sql}", r.memory.question))
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
            Err(_) => String::new(),
        };

        // 3. Guardrails (dry-plan).
        let guards = Guardrails {
            enforce_read_only: self.config.enforce_read_only,
            allowed_tables: self.semantic.allowed_tables(),
            default_limit: self.config.default_limit,
        };

        // 4. Generate → validate → execute, with a multi-candidate first pass
        //    (self-consistency) followed by execution-guided self-correction.
        let candidates = self.config.candidates.max(1);
        let mut feedback: Option<(String, String)> = None;

        // First pass: try up to `candidates` diverse candidates; return the
        // first that passes guardrails and executes.
        for variant in 0..candidates as usize {
            let sql = extract_sql(
                &self
                    .generate(question, &schema_context, &examples, None, variant)
                    .await,
            );
            match self.validate_and_run(&guards, &sql, context).await {
                Ok((safe_sql, df)) => {
                    return self.success(context, question, &safe_sql, df, 0).await
                }
                Err(reason) => feedback = Some((sql, reason)),
            }
        }

        // Correction passes: single-shot, fed the prior error.
        for attempt in 1..=self.config.max_correction_attempts {
            let sql = extract_sql(
                &self
                    .generate(question, &schema_context, &examples, feedback.as_ref(), 0)
                    .await,
            );
            match self.validate_and_run(&guards, &sql, context).await {
                Ok((safe_sql, df)) => {
                    return self
                        .success(context, question, &safe_sql, df, attempt)
                        .await
                }
                Err(reason) => feedback = Some((sql, reason)),
            }
        }

        let detail = feedback.map(|f| f.1).unwrap_or_default();
        ToolResult::error(format!(
            "Could not produce valid SQL after {candidates} candidate(s) and \
             {} correction(s). Last issue: {detail}",
            self.config.max_correction_attempts
        ))
    }

    /// Guardrail-check then execute `sql`; `Ok((safe_sql, df))` on success, or
    /// `Err(reason)` describing the guardrail rejection or execution error.
    async fn validate_and_run(
        &self,
        guards: &Guardrails,
        sql: &str,
        context: &ToolContext,
    ) -> std::result::Result<(String, gauss_engine::dataframe::DataFrame), String> {
        match guards.check_and_fix(sql) {
            Ok(safe_sql) => match self.runner.run_sql(&safe_sql, context).await {
                Ok(df) => Ok((safe_sql, df)),
                Err(e) => Err(format!("execution error: {e}")),
            },
            Err(g) => Err(format!("guardrail rejection: {g}")),
        }
    }

    async fn success(
        &self,
        context: &ToolContext,
        question: &str,
        sql: &str,
        mut df: gauss_engine::dataframe::DataFrame,
        corrections: u32,
    ) -> ToolResult {
        // PII-aware redaction: mask PII-flagged columns unless the user is in a
        // PII-access group. Applied once so both the UI table and the LLM-facing
        // summary are redacted.
        let mut redacted_cols: Vec<String> = Vec::new();
        if self.config.redact_pii {
            let user_has_access = context
                .user
                .group_memberships
                .iter()
                .any(|g| self.config.pii_access_groups.contains(g));
            if !user_has_access {
                let pii: HashSet<String> = self
                    .semantic
                    .pii_columns()
                    .into_iter()
                    .map(|(_, c)| c.to_lowercase())
                    .collect();
                for (idx, col) in df.columns.iter().enumerate() {
                    if pii.contains(&col.to_lowercase()) {
                        redacted_cols.push(col.clone());
                        for row in &mut df.rows {
                            if let Some(cell) = row.get_mut(idx) {
                                *cell = json!("***");
                            }
                        }
                    }
                }
            }
        }

        let row_count = df.row_count();
        let columns = df.columns.clone();

        // Self-learning: remember the successful question → SQL.
        let mut args = Map::new();
        args.insert("sql".into(), json!(sql));
        let _ = context
            .agent_memory
            .save_tool_usage(question, "text_to_sql", &args, context, true, None)
            .await;

        let component = UiComponent::new(RichComponent::dataframe(
            df.to_records(),
            columns.clone(),
            Some("Query Results".to_string()),
        ));

        let mut note = if corrections > 0 {
            format!(" (after {corrections} self-correction(s))")
        } else {
            String::new()
        };
        if !redacted_cols.is_empty() {
            note.push_str(&format!(
                " [PII columns redacted for this user: {}]",
                redacted_cols.join(", ")
            ));
        }
        let preview = {
            let mut head = df;
            head.rows.truncate(20);
            head.to_csv().trim_end().to_string()
        };

        ToolResult::success(format!(
            "Generated SQL{note}:\n```sql\n{sql}\n```\nReturned {row_count} row(s) with columns \
             [{}].\n{preview}",
            columns.join(", ")
        ))
        .with_ui(component)
    }
}

#[async_trait]
impl Tool for TextToSqlTool {
    type Args = TextToSqlArgs;

    fn name(&self) -> &str {
        "text_to_sql"
    }

    fn description(&self) -> &str {
        "Answer a natural-language question about the data by generating and running a \
         validated, read-only SQL query grounded in the semantic model. Prefer this over \
         run_sql for analytical questions."
    }

    fn access_groups(&self) -> Vec<String> {
        self.access_groups.clone()
    }

    async fn execute(&self, context: &ToolContext, args: TextToSqlArgs) -> ToolResult {
        self.run_pipeline(context, &args.question).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauss_engine::error::Result;
    use gauss_engine::model::llm::LlmResponse;
    use std::sync::Mutex;

    /// An LLM that returns a queued script of responses in order.
    struct ScriptedLlm {
        responses: Mutex<std::collections::VecDeque<String>>,
    }
    impl ScriptedLlm {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(String::from).collect()),
            }
        }
    }
    #[async_trait]
    impl LlmService for ScriptedLlm {
        async fn send_request(&self, _request: LlmRequest) -> Result<LlmResponse> {
            let next = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| "SELECT 1".to_string());
            Ok(LlmResponse {
                content: Some(next),
                ..Default::default()
            })
        }
    }

    fn semantic() -> Arc<SemanticModel> {
        Arc::new(
            SemanticModel::from_yaml(
                "models:\n  - name: customers\n    columns:\n      - name: id\n        type: INTEGER\n      - name: name\n        type: TEXT\n",
            )
            .unwrap(),
        )
    }

    fn sqlite_runner() -> Arc<dyn SqlRunner> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "gauss_textsql_{}_{}.db",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT);
             INSERT INTO customers (name) VALUES ('Acme'), ('Globex');",
        )
        .unwrap();
        Arc::new(gauss_sql::SqliteRunner::new(
            path.to_string_lossy().into_owned(),
        ))
    }

    fn ctx() -> ToolContext {
        ctx_with_groups(&["admin"])
    }

    fn ctx_with_groups(groups: &[&str]) -> ToolContext {
        use gauss_engine::defaults::InMemoryAgentMemory;
        ToolContext::new(
            gauss_engine::model::user::User::new("u").with_groups(groups.iter().copied()),
            "c",
            "r",
            Arc::new(InMemoryAgentMemory::new()),
        )
    }

    fn semantic_pii() -> Arc<SemanticModel> {
        Arc::new(
            SemanticModel::from_yaml(
                "models:\n  - name: customers\n    columns:\n      - name: id\n        type: INTEGER\n      - name: name\n        type: TEXT\n        is_pii: true\n",
            )
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn value_linking_grounds_filters_in_real_values() {
        // Flag `name` for value linking; the pipeline should fetch its real
        // distinct values from the DB and surface them to the model.
        let semantic = Arc::new(
            SemanticModel::from_yaml(
                "models:\n  - name: customers\n    columns:\n      - name: name\n        type: TEXT\n        link_values: true\n",
            )
            .unwrap(),
        );
        let llm = Arc::new(ScriptedLlm::new(vec!["SELECT 1"]));
        let tool = TextToSqlTool::new(llm, sqlite_runner(), semantic);
        let hints = tool.value_hints(&ctx(), &["customers".to_string()]).await;
        assert!(hints.contains("customers.name values"), "{hints}");
        assert!(
            hints.contains("Acme") && hints.contains("Globex"),
            "{hints}"
        );

        // Disabling value linking yields no hints.
        let tool = tool.with_config(TextToSqlConfig {
            value_linking: false,
            ..Default::default()
        });
        assert!(tool
            .value_hints(&ctx(), &["customers".to_string()])
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn multi_candidate_selects_first_valid_in_one_pass() {
        // candidates=2: the first candidate fails to execute, the second works;
        // selection happens in the first pass (no self-correction reported).
        let llm = Arc::new(ScriptedLlm::new(vec![
            "```sql\nSELECT nonexistent FROM customers\n```",
            "```sql\nSELECT name FROM customers\n```",
        ]));
        let tool =
            TextToSqlTool::new(llm, sqlite_runner(), semantic()).with_config(TextToSqlConfig {
                candidates: 2,
                ..Default::default()
            });
        let r = tool.run_pipeline(&ctx(), "names").await;
        assert!(r.success, "{:?}", r.error);
        assert!(r.result_for_llm.contains("Acme"));
        // Picked a valid candidate directly — not via the correction loop.
        assert!(
            !r.result_for_llm.contains("self-correction"),
            "{}",
            r.result_for_llm
        );
    }

    #[tokio::test]
    async fn happy_path_generates_and_runs() {
        let llm = Arc::new(ScriptedLlm::new(vec![
            "```sql\nSELECT name FROM customers\n```",
        ]));
        let tool = TextToSqlTool::new(llm, sqlite_runner(), semantic());
        let r = tool.run_pipeline(&ctx(), "list customer names").await;
        assert!(r.success, "{:?}", r.error);
        assert!(r.result_for_llm.contains("Acme"));
        // LIMIT was injected by the guardrails.
        assert!(r.result_for_llm.to_lowercase().contains("limit 1000"));
    }

    #[tokio::test]
    async fn ambiguous_question_triggers_clarification() {
        // With clarification on, the first LLM call returns CLARIFY → no SQL runs.
        let llm = Arc::new(ScriptedLlm::new(vec![
            "CLARIFY: Which time period? | Which revenue metric?",
        ]));
        let tool = TextToSqlTool::new(llm, sqlite_runner(), semantic()).with_clarification(true);
        let r = tool.run_pipeline(&ctx(), "show me the best ones").await;
        assert!(r.success);
        assert!(r.result_for_llm.contains("clarifying"));
        assert!(r.result_for_llm.contains("Which time period?"));
        // No data was queried (no rows surfaced).
        assert!(!r.result_for_llm.contains("Acme"));
    }

    #[tokio::test]
    async fn clear_question_skips_clarification_and_runs() {
        // `OK` verdict → pipeline proceeds to generation + execution.
        let llm = Arc::new(ScriptedLlm::new(vec![
            "OK",
            "```sql\nSELECT name FROM customers\n```",
        ]));
        let tool = TextToSqlTool::new(llm, sqlite_runner(), semantic()).with_clarification(true);
        let r = tool.run_pipeline(&ctx(), "list all customer names").await;
        assert!(r.success, "{:?}", r.error);
        assert!(r.result_for_llm.contains("Acme"));
    }

    #[tokio::test]
    async fn self_corrects_after_execution_error() {
        // First SQL references a bad column → execution error → correction.
        let llm = Arc::new(ScriptedLlm::new(vec![
            "```sql\nSELECT nonexistent FROM customers\n```",
            "```sql\nSELECT name FROM customers\n```",
        ]));
        let tool = TextToSqlTool::new(llm, sqlite_runner(), semantic());
        let r = tool.run_pipeline(&ctx(), "names please").await;
        assert!(r.success, "{:?}", r.error);
        assert!(r.result_for_llm.contains("self-correction"));
    }

    #[tokio::test]
    async fn guardrail_blocks_then_corrects() {
        // First attempt is a non-SELECT (blocked by guardrails), then a valid SELECT.
        let llm = Arc::new(ScriptedLlm::new(vec![
            "```sql\nDELETE FROM customers\n```",
            "```sql\nSELECT name FROM customers\n```",
        ]));
        let tool = TextToSqlTool::new(llm, sqlite_runner(), semantic());
        let r = tool.run_pipeline(&ctx(), "remove things").await;
        assert!(r.success, "{:?}", r.error);
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let llm = Arc::new(ScriptedLlm::new(vec![
            "```sql\nDROP TABLE customers\n```",
            "```sql\nDROP TABLE customers\n```",
            "```sql\nDROP TABLE customers\n```",
        ]));
        let tool = TextToSqlTool::new(llm, sqlite_runner(), semantic());
        let r = tool.run_pipeline(&ctx(), "destroy").await;
        assert!(!r.success);
    }

    #[tokio::test]
    async fn pii_redacted_for_unprivileged_user() {
        let llm = Arc::new(ScriptedLlm::new(vec![
            "```sql\nSELECT name FROM customers\n```",
        ]));
        let tool = TextToSqlTool::new(llm, sqlite_runner(), semantic_pii());
        // admin lacks the `view_pii` group, so PII is masked.
        let r = tool
            .run_pipeline(&ctx_with_groups(&["admin"]), "names")
            .await;
        assert!(r.success, "{:?}", r.error);
        assert!(r.result_for_llm.contains("***"), "{}", r.result_for_llm);
        assert!(!r.result_for_llm.contains("Acme"), "{}", r.result_for_llm);
        assert!(r.result_for_llm.contains("PII columns redacted"));
    }

    #[tokio::test]
    async fn pii_visible_for_privileged_user() {
        let llm = Arc::new(ScriptedLlm::new(vec![
            "```sql\nSELECT name FROM customers\n```",
        ]));
        let tool = TextToSqlTool::new(llm, sqlite_runner(), semantic_pii());
        let r = tool
            .run_pipeline(&ctx_with_groups(&["view_pii"]), "names")
            .await;
        assert!(r.success, "{:?}", r.error);
        assert!(r.result_for_llm.contains("Acme"), "{}", r.result_for_llm);
        assert!(!r.result_for_llm.contains("***"));
    }

    #[test]
    fn extract_sql_variants() {
        assert_eq!(extract_sql("```sql\nSELECT 1\n```"), "SELECT 1");
        assert_eq!(extract_sql("```\nSELECT 2\n```"), "SELECT 2");
        assert_eq!(extract_sql("SELECT 3"), "SELECT 3");
    }
}
