//! Runtime assembly helpers shared by the `gauss-chat` (server) and `gauss-chat-tui`
//! binaries: LLM provider selection and demo-database seeding. Keeping these in
//! one place avoids drift between the two front-ends.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::ValueEnum;
use gauss_engine::traits::LlmService;
use gauss_llm::{
    AnthropicLlmService, GeminiLlmService, MockLlmService, OllamaLlmService, OpenAiLlmService,
    VllmLlmService,
};

/// Selectable LLM backend. `Mock` needs no credentials; cloud providers read
/// their key from the environment; `Vllm` targets an on-prem OpenAI-compatible
/// server via `base_url`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Provider {
    Mock,
    Ollama,
    Openai,
    Anthropic,
    Gemini,
    Vllm,
}

/// Construct an [`LlmService`] for `provider`, applying an optional model
/// override and base URL. Returns a clear error if a required key or argument
/// is missing.
pub fn build_llm(
    provider: Provider,
    model: Option<String>,
    base_url: Option<String>,
) -> Result<Arc<dyn LlmService>> {
    Ok(match provider {
        Provider::Mock => Arc::new(MockLlmService::new()),
        Provider::Ollama => {
            let mut svc = OllamaLlmService::new(model.unwrap_or_else(|| "llama3.1".into()));
            if let Some(url) = base_url {
                svc = svc.with_base_url(url);
            }
            Arc::new(svc)
        }
        Provider::Openai => {
            let key = std::env::var("OPENAI_API_KEY")
                .context("OPENAI_API_KEY must be set for --llm openai")?;
            let mut svc = OpenAiLlmService::new(key, model.unwrap_or_else(|| "gpt-4o-mini".into()));
            if let Some(url) = base_url {
                svc = svc.with_base_url(url);
            }
            Arc::new(svc)
        }
        Provider::Anthropic => {
            let key = std::env::var("ANTHROPIC_API_KEY")
                .context("ANTHROPIC_API_KEY must be set for --llm anthropic")?;
            Arc::new(AnthropicLlmService::new(
                key,
                model.unwrap_or_else(|| "claude-sonnet-4-5".into()),
            ))
        }
        Provider::Gemini => {
            let key = std::env::var("GEMINI_API_KEY")
                .or_else(|_| std::env::var("GOOGLE_API_KEY"))
                .context("GEMINI_API_KEY (or GOOGLE_API_KEY) must be set for --llm gemini")?;
            Arc::new(GeminiLlmService::new(
                key,
                model.unwrap_or_else(|| "gemini-2.0-flash".into()),
            ))
        }
        Provider::Vllm => {
            let base = base_url.context(
                "--base-url (the vLLM OpenAI endpoint, e.g. http://host:8000/v1) is required for --llm vllm",
            )?;
            let model = model.context("--model is required for --llm vllm")?;
            match std::env::var("VLLM_API_KEY") {
                Ok(key) => Arc::new(VllmLlmService::with_api_key(base, key, model)),
                Err(_) => Arc::new(VllmLlmService::new(base, model)),
            }
        }
    })
}

/// Seed a small sample SQLite database (customers + orders) so the demo is
/// usable immediately with no external data source.
pub fn seed_sample_db(path: &str) -> Result<()> {
    let conn = rusqlite::Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT, country TEXT, lifetime_value REAL);
         INSERT INTO customers (name, country, lifetime_value) VALUES
            ('Acme Corp', 'US', 152000.0), ('Globex', 'DE', 98000.0),
            ('Initech', 'US', 45000.0), ('Umbrella', 'UK', 210000.0),
            ('Hooli', 'US', 320000.0);
         CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER, amount REAL, status TEXT);
         INSERT INTO orders (customer_id, amount, status) VALUES
            (1, 1200.0, 'paid'), (1, 800.0, 'paid'), (2, 500.0, 'pending'),
            (5, 9000.0, 'paid'), (4, 3000.0, 'refunded');",
    )?;
    Ok(())
}
