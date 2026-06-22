//! GaussAnalytics server runner.
//!
//! Wires capabilities into an agent and serves the SSE chat API. LLM providers
//! (mock / Ollama / OpenAI / Anthropic / Gemini / vLLM) and the demo-DB seeding
//! come from `gauss-runtime`, shared with the TUI. Adds a SQLite database, a
//! sandboxed local file system, the run_sql / visualize_data / file-system /
//! python / memory tools, optional `text_to_sql` (with a semantic model),
//! pluggable conversation stores, audit logging, and header-based auth.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use gauss_embed::{Embedder, HashingEmbedder, OllamaEmbedder, OpenAiEmbedder};
use gauss_engine::agent::AgentBuilder;
use gauss_engine::defaults::{
    FileAuditLogger, FileConversationStore, HeaderUserResolver, InMemoryAgentMemory,
    InMemoryConversationStore, LocalFileSystem, StaticUserResolver, TracingObservabilityProvider,
};
use gauss_engine::enhancer::DefaultLlmContextEnhancer;
use gauss_engine::recovery::RetryStrategy;
use gauss_engine::tool::ToolRegistry;
use gauss_engine::traits::{
    AgentMemory, AuditLogger, ConversationStore, FileSystem, LlmContextEnhancer,
    ObservabilityProvider, SqlRunner, UserResolver,
};
use gauss_memory::InMemoryVectorMemory;
use gauss_runtime::{build_llm, seed_sample_db, Provider};
use gauss_sql::SqliteRunner;
use gauss_textsql::TextToSqlTool;
use gauss_tools::{
    ListFilesTool, PipInstallTool, ReadFileTool, RunPythonFileTool, RunSqlTool,
    SaveQuestionToolArgsTool, SaveTextMemoryTool, SchemaContextEnhancer, SearchFilesTool,
    SearchSavedCorrectToolUsesTool, VisualizeDataTool, WriteFileTool,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Store {
    Memory,
    File,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Auth {
    /// Single local admin user (development / single-tenant).
    Static,
    /// Resolve identity from request headers injected by an upstream gateway.
    Header,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Memory {
    /// In-memory lexical similarity (no embeddings).
    Lexical,
    /// In-process vector memory with cosine search over embeddings.
    Vector,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum EmbedderKind {
    /// Offline deterministic feature hashing (no network).
    Hashing,
    Ollama,
    Openai,
}

#[derive(Parser, Debug)]
#[command(name = "gaussanalytics", about = "GaussAnalytics NL2SQL agent server")]
struct Args {
    /// LLM provider.
    #[arg(long, value_enum, default_value_t = Provider::Mock, env = "GAUSS_CHAT_LLM")]
    llm: Provider,

    /// Model name (provider-specific). Defaults chosen per provider if unset.
    #[arg(long, env = "GAUSS_CHAT_MODEL")]
    model: Option<String>,

    /// Base URL for Ollama (or an OpenAI-compatible endpoint).
    #[arg(long, env = "GAUSS_CHAT_LLM_BASE_URL")]
    base_url: Option<String>,

    /// Path to the SQLite database. Seeded with sample data if it does not exist.
    #[arg(long, default_value = "gauss_demo.db", env = "GAUSS_CHAT_DB")]
    db: String,

    /// Directory the file-system tools are sandboxed to (for query CSVs, charts).
    #[arg(long, default_value = "./gauss_data", env = "GAUSS_CHAT_DATA_DIR")]
    data_dir: String,

    /// Identity resolution: `static` (single local admin) or `header`
    /// (read `x-user-id`/`x-user-email`/`x-user-groups` from an upstream gateway).
    #[arg(long, value_enum, default_value_t = Auth::Static)]
    auth: Auth,

    /// Conversation store backend.
    #[arg(long, value_enum, default_value_t = Store::Memory)]
    store: Store,

    /// Directory for the file conversation store (when --store file).
    #[arg(long, default_value = "./gauss_conversations")]
    store_dir: String,

    /// If set, write audit events as JSONL to this file.
    #[arg(long, env = "GAUSS_CHAT_AUDIT_LOG")]
    audit_log: Option<String>,

    /// Agent memory backend.
    #[arg(long, value_enum, default_value_t = Memory::Lexical)]
    memory: Memory,

    /// Embedder for vector memory (used when --memory vector).
    #[arg(long, value_enum, default_value_t = EmbedderKind::Hashing)]
    embedder: EmbedderKind,

    /// Embedding model name (for ollama/openai embedders).
    #[arg(long)]
    embed_model: Option<String>,

    /// Embedding dimension (defaults per embedder).
    #[arg(long)]
    embed_dim: Option<usize>,

    /// Retry LLM calls up to N attempts (with a short delay) on transient errors.
    #[arg(long, default_value_t = 1)]
    retry_llm: u32,

    /// Cache identical LLM requests in memory to cut cost and latency.
    #[arg(long)]
    cache_llm: bool,

    /// Path to a semantic-model YAML/JSON file. Enables the grounded,
    /// self-correcting `text_to_sql` tool.
    #[arg(long, env = "GAUSS_CHAT_SEMANTIC_MODEL")]
    semantic_model: Option<String>,

    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    #[arg(long, default_value_t = 8000, env = "GAUSS_CHAT_PORT")]
    port: u16,
}

fn build_embedder(args: &Args) -> Result<Arc<dyn Embedder>> {
    Ok(match args.embedder {
        EmbedderKind::Hashing => Arc::new(HashingEmbedder::new(args.embed_dim.unwrap_or(256))),
        EmbedderKind::Ollama => {
            let model = args
                .embed_model
                .clone()
                .unwrap_or_else(|| "nomic-embed-text".into());
            let mut e = OllamaEmbedder::new(model, args.embed_dim.unwrap_or(768));
            if let Ok(url) = std::env::var("GAUSS_CHAT_EMBED_BASE_URL") {
                e = e.with_base_url(url);
            }
            Arc::new(e)
        }
        EmbedderKind::Openai => {
            let key = std::env::var("OPENAI_API_KEY")
                .context("OPENAI_API_KEY must be set for --embedder openai")?;
            let model = args
                .embed_model
                .clone()
                .unwrap_or_else(|| "text-embedding-3-small".into());
            Arc::new(OpenAiEmbedder::new(
                key,
                model,
                args.embed_dim.unwrap_or(1536),
            ))
        }
    })
}

fn build_memory(args: &Args) -> Result<Arc<dyn AgentMemory>> {
    Ok(match args.memory {
        Memory::Lexical => Arc::new(InMemoryAgentMemory::new()),
        Memory::Vector => Arc::new(InMemoryVectorMemory::new(build_embedder(args)?)),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();

    if args.db != ":memory:" && !Path::new(&args.db).exists() {
        tracing::info!(db = %args.db, "seeding sample database");
        seed_sample_db(&args.db)?;
    }
    std::fs::create_dir_all(&args.data_dir).ok();

    // Capabilities.
    let runner: Arc<dyn SqlRunner> = Arc::new(SqliteRunner::new(args.db.clone()));
    let file_system: Arc<dyn FileSystem> = Arc::new(LocalFileSystem::new(&args.data_dir));
    let agent_memory: Arc<dyn AgentMemory> = build_memory(&args)?;
    let mut llm = build_llm(args.llm, args.model.clone(), args.base_url.clone())?;
    if args.cache_llm {
        llm = Arc::new(gauss_llm::CachingLlmService::new(llm));
    }
    let user_resolver: Arc<dyn UserResolver> = match args.auth {
        Auth::Static => Arc::new(StaticUserResolver::admin()),
        Auth::Header => Arc::new(HeaderUserResolver::new()),
    };

    // Tools.
    let mut registry = ToolRegistry::new();
    registry.register(RunSqlTool::new(runner.clone()).with_file_system(file_system.clone()));
    registry.register(VisualizeDataTool::new(file_system.clone()));
    registry.register(SearchSavedCorrectToolUsesTool);
    registry.register(SaveQuestionToolArgsTool);
    registry.register(SaveTextMemoryTool);
    // File-system + python tools (write/python are admin-gated by the tools).
    registry.register(ListFilesTool::new(file_system.clone()));
    registry.register(ReadFileTool::new(file_system.clone()));
    registry.register(WriteFileTool::new(file_system.clone()));
    registry.register(SearchFilesTool::new(file_system.clone()));
    registry.register(RunPythonFileTool::new(file_system.clone()));
    registry.register(PipInstallTool::new(file_system.clone()));

    // Semantic layer → grounded, self-correcting text_to_sql tool.
    if let Some(path) = &args.semantic_model {
        let model = gauss_semantic::SemanticModel::from_path(path)
            .map_err(|e| anyhow::anyhow!("load semantic model: {e}"))?;
        tracing::info!(
            models = model.models.len(),
            "semantic model loaded; text_to_sql enabled"
        );
        registry.register(TextToSqlTool::new(
            llm.clone(),
            runner.clone(),
            Arc::new(model),
        ));
    }

    // Context enhancement: inject the live DB schema (so the LLM can query any
    // table, including CSVs uploaded at runtime) and chain RAG over saved
    // text memories underneath it.
    let rag: Arc<dyn LlmContextEnhancer> =
        Arc::new(DefaultLlmContextEnhancer::new(agent_memory.clone()));
    let enhancer: Arc<dyn LlmContextEnhancer> =
        Arc::new(SchemaContextEnhancer::new(runner.clone()).with_inner(rag));

    // Conversation store.
    let store: Arc<dyn ConversationStore> = match args.store {
        Store::Memory => Arc::new(InMemoryConversationStore::new()),
        Store::File => Arc::new(FileConversationStore::new(&args.store_dir)),
    };

    // Observability (tracing-backed).
    let observability: Arc<dyn ObservabilityProvider> = Arc::new(TracingObservabilityProvider);

    let mut builder = AgentBuilder::new(llm, registry, user_resolver, agent_memory)
        .llm_context_enhancer(enhancer)
        .conversation_store(store)
        .observability_provider(observability);

    // Error recovery: retry transient LLM failures.
    if args.retry_llm > 1 {
        builder =
            builder.error_recovery_strategy(Arc::new(RetryStrategy::new(args.retry_llm, 250)));
    }

    // Optional audit logging.
    if let Some(path) = &args.audit_log {
        let audit: Arc<dyn AuditLogger> = Arc::new(FileAuditLogger::new(path.clone()));
        tracing::info!(audit_log = %path, "audit logging enabled");
        builder = builder.audit_logger(audit);
    }

    let agent = Arc::new(builder.build());

    let addr = format!("{}:{}", args.host, args.port);
    tracing::info!(provider = ?args.llm, "GaussAnalytics listening on http://{addr}");
    println!("GaussAnalytics ready ({:?}) → open http://{addr}", args.llm);
    gauss_chat::serve_with_db(agent, args.db.clone(), &addr).await?;
    Ok(())
}
