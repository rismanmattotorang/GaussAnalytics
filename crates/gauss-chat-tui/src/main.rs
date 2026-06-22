//! GaussAnalytics terminal UI.
//!
//! A full-screen ratatui chat client wired to an in-process [`Agent`]: type a
//! question, watch the agent's components (status, tasks, tables, answers)
//! stream into the transcript. Same agent/tools/runner as the server, so it
//! exercises the real backend path without a network hop.

mod render;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;
use tokio::sync::mpsc;

use gauss_engine::agent::AgentBuilder;
use gauss_engine::defaults::{InMemoryAgentMemory, StaticUserResolver};
use gauss_engine::model::user::RequestContext;
use gauss_engine::tool::ToolRegistry;
use gauss_engine::traits::{AgentMemory, LlmContextEnhancer, SqlRunner, UserResolver};
use gauss_engine::Agent;
use gauss_runtime::{build_llm, seed_sample_db, Provider};
use gauss_sql::SqliteRunner;
use gauss_tools::{RunSqlTool, SchemaContextEnhancer};

use render::{render_component, Rendered, BRAND, BRAND_DEEP};

#[derive(Parser, Debug)]
#[command(name = "gauss-chat-tui", about = "GaussAnalytics terminal chat client")]
struct Args {
    #[arg(long, value_enum, default_value_t = Provider::Mock, env = "GAUSS_CHAT_LLM")]
    llm: Provider,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long, default_value = "gauss_demo.db", env = "GAUSS_CHAT_DB")]
    db: String,
}

/// Messages from the streaming agent task to the UI loop.
enum Msg {
    Lines(Vec<Line<'static>>),
    Status(String),
    Done,
}

/// Frames for the busy spinner (Braille spinner, advanced on each timer tick).
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

struct App {
    agent: Arc<Agent>,
    /// SQLite path, so `/load` can ingest CSVs into queryable tables.
    db_path: String,
    transcript: Vec<Line<'static>>,
    input: String,
    status: String,
    busy: bool,
    conversation_id: String,
    /// Lines scrolled up from the bottom. `0` follows the newest content.
    scroll: usize,
    /// Animation frame for the busy spinner.
    spinner: usize,
}

impl App {
    fn new(agent: Arc<Agent>, db_path: String) -> Self {
        let transcript = vec![
            Line::from(Span::styled(
                "GaussAnalytics",
                Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "by Gaussian Technologies — ask a question about your data. Esc to quit.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "Tip: /load <file.csv> [table] imports a CSV you can then query in plain English.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        Self {
            agent,
            db_path,
            transcript,
            input: String::new(),
            status: "Ready".to_string(),
            busy: false,
            conversation_id: "tui-session".to_string(),
            scroll: 0,
            spinner: 0,
        }
    }

    /// Append a styled line to the transcript (used by slash commands).
    fn push(&mut self, line: Line<'static>) {
        self.transcript.push(line);
    }

    /// Handle a `/command`. Slash commands run locally and never hit the agent.
    fn handle_command(&mut self, rest: &str) {
        let mut parts = rest.split_whitespace();
        match parts.next() {
            Some("load") | Some("import") => {
                let path = parts.next();
                let table = parts.next();
                self.load_csv(path, table);
            }
            Some("help") => {
                self.push(
                    Span::styled(
                        "Commands: /load <file.csv> [table] · /help · Esc quit",
                        Style::default().fg(BRAND),
                    )
                    .into(),
                );
            }
            other => {
                self.push(
                    Span::styled(
                        format!("Unknown command: /{}", other.unwrap_or("")),
                        Style::default().fg(Color::Red),
                    )
                    .into(),
                );
            }
        }
    }

    /// Read a CSV from disk and ingest it into the SQLite database.
    fn load_csv(&mut self, path: Option<&str>, table: Option<&str>) {
        let Some(path) = path else {
            self.push(
                Span::styled(
                    "usage: /load <file.csv> [table]",
                    Style::default().fg(Color::DarkGray),
                )
                .into(),
            );
            return;
        };
        let table = table.map_or_else(
            || {
                std::path::Path::new(path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("uploaded_table")
                    .to_string()
            },
            str::to_string,
        );
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) => {
                self.push(
                    Span::styled(
                        format!("✗ cannot read {path}: {e}"),
                        Style::default().fg(Color::Red),
                    )
                    .into(),
                );
                return;
            }
        };
        match gauss_sql::ingest_csv(&self.db_path, &table, &data) {
            Ok(s) => {
                self.push(
                    Span::styled(
                        format!(
                            "✓ Imported {path} → table {} ({} rows)",
                            s.table, s.row_count
                        ),
                        Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
                    )
                    .into(),
                );
                let cols = s
                    .columns
                    .iter()
                    .map(|c| format!("{} {}", c.name, c.sql_type))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.push(
                    Span::styled(
                        format!("  columns: {cols}"),
                        Style::default().fg(Color::DarkGray),
                    )
                    .into(),
                );
                self.push(
                    Span::styled(
                        format!("  now ask, e.g. \"show me a sample of {}\"", s.table),
                        Style::default().fg(Color::DarkGray),
                    )
                    .into(),
                );
            }
            Err(e) => {
                self.push(
                    Span::styled(
                        format!("✗ import failed: {e}"),
                        Style::default().fg(Color::Red),
                    )
                    .into(),
                );
            }
        }
    }

    fn submit(&mut self, tx: &mpsc::UnboundedSender<Msg>) {
        let q = self.input.trim().to_string();
        if q.is_empty() || self.busy {
            return;
        }
        // Local slash commands (e.g. /load a CSV) never go to the agent.
        if let Some(rest) = q.strip_prefix('/') {
            self.input.clear();
            self.scroll = 0;
            self.handle_command(rest);
            return;
        }
        self.transcript.push(Line::from(Span::styled(
            format!("You: {q}"),
            Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
        )));
        self.input.clear();
        self.busy = true;
        self.scroll = 0; // follow the new answer as it streams in
        self.status = "Working…".to_string();

        let agent = self.agent.clone();
        let conv = self.conversation_id.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut stream = Box::pin(agent.send_message(RequestContext::default(), q, Some(conv)));
            while let Some(component) = stream.next().await {
                let rich = component.rich_component.serialize_for_frontend();
                let simple = component
                    .simple_component
                    .as_ref()
                    .map(gauss_engine::SimpleComponent::serialize_for_frontend);
                match render_component(&rich, simple.as_ref()) {
                    Rendered::Lines(ls) => {
                        let _ = tx.send(Msg::Lines(ls));
                    }
                    Rendered::Status(s) => {
                        let _ = tx.send(Msg::Status(s));
                    }
                    Rendered::Skip => {}
                }
            }
            let _ = tx.send(Msg::Done);
        });
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Lines(ls) => {
                // If the user has scrolled up to read history, keep that region
                // anchored as new lines arrive; otherwise stay pinned to bottom.
                if self.scroll > 0 {
                    self.scroll += ls.len();
                }
                self.transcript.extend(ls);
            }
            Msg::Status(s) => self.status = s,
            Msg::Done => {
                self.busy = false;
                self.status = "Ready".to_string();
            }
        }
    }
}

/// The slice of `transcript` to display, given the wrap `width`, viewport
/// `height`, and how many lines the user has scrolled up from the bottom.
struct Viewport<'a> {
    lines: Vec<Line<'a>>,
    /// `scroll` clamped to the valid range — write this back to the app.
    scroll: usize,
    /// Furthest the user can scroll up (0 when everything already fits).
    max_scroll: usize,
}

/// Compute the visible window of `lines` that fits `height` rows at `width`,
/// accounting for wrapping. `scroll` lines are dropped from the bottom (0 keeps
/// the newest content visible); it is clamped so the view never runs off the top.
fn viewport<'a>(lines: &'a [Line<'a>], width: usize, height: usize, scroll: usize) -> Viewport<'a> {
    let w = width.max(1);
    let h = height.max(1);
    let line_rows = |l: &Line| -> usize {
        let len: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
        len.div_ceil(w).max(1)
    };

    // How many whole lines fit when anchored to the very bottom.
    let mut rows = 0usize;
    let mut fit = 0usize;
    let mut i = lines.len();
    while i > 0 {
        let lh = line_rows(&lines[i - 1]);
        if rows + lh > h {
            break;
        }
        rows += lh;
        fit += 1;
        i -= 1;
    }
    let max_scroll = lines.len().saturating_sub(fit);
    let scroll = scroll.min(max_scroll);

    // Fill `height` rows upward, ending `scroll` lines above the bottom.
    let end = lines.len() - scroll;
    let mut rows = 0usize;
    let mut start = end;
    while start > 0 {
        let lh = line_rows(&lines[start - 1]);
        if rows + lh > h {
            break;
        }
        rows += lh;
        start -= 1;
    }
    Viewport {
        lines: lines[start..end].to_vec(),
        scroll,
        max_scroll,
    }
}

fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .split(f.area());

    // Transcript (scroll-aware viewport).
    let inner_w = chunks[0].width.saturating_sub(2) as usize;
    let inner_h = chunks[0].height.saturating_sub(2) as usize;
    let view = viewport(&app.transcript, inner_w, inner_h, app.scroll);
    app.scroll = view.scroll; // clamp back
    let title = if app.scroll > 0 {
        format!(
            " GaussAnalytics ▲ {}/{} lines above — End to jump down ",
            app.scroll, view.max_scroll
        )
    } else {
        " GaussAnalytics ".to_string()
    };
    f.render_widget(
        Paragraph::new(Text::from(view.lines))
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(BRAND_DEEP))
                    .title(Span::styled(title, Style::default().fg(BRAND))),
            )
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    // Status line, with an animated spinner while the agent is working.
    let (marker, status_style) = if app.busy {
        (
            SPINNER[app.spinner % SPINNER.len()],
            Style::default().fg(BRAND),
        )
    } else {
        ("●", Style::default().fg(Color::Green))
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{marker} {}", app.status),
            status_style,
        ))),
        chunks[1],
    );

    // Input, with a block cursor.
    f.render_widget(
        Paragraph::new(format!("> {}▏", app.input)).block(
            Block::bordered()
                .border_style(Style::default().fg(BRAND_DEEP))
                .title(" Ask (Enter to send) "),
        ),
        chunks[2],
    );

    // Footer keybinding hints.
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Esc quit · Enter send · /load <file.csv> import · ↑/↓ PgUp/PgDn scroll · End latest",
            Style::default().fg(Color::DarkGray),
        ))),
        chunks[3],
    );
}

async fn run(mut terminal: ratatui::DefaultTerminal, mut app: App) -> Result<()> {
    let mut events = EventStream::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    // Drives the busy spinner animation; only consulted while `app.busy`.
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(120));

    loop {
        terminal.draw(|f| draw(f, &mut app))?;
        tokio::select! {
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        match key.code {
                            KeyCode::Esc => break,
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                            KeyCode::Enter => app.submit(&tx),
                            KeyCode::Backspace => { app.input.pop(); }
                            // Scrollback (clamped against the viewport in `draw`).
                            KeyCode::Up => app.scroll += 1,
                            KeyCode::Down => app.scroll = app.scroll.saturating_sub(1),
                            KeyCode::PageUp => app.scroll += 10,
                            KeyCode::PageDown => app.scroll = app.scroll.saturating_sub(10),
                            KeyCode::Home => app.scroll = app.transcript.len(),
                            KeyCode::End => app.scroll = 0,
                            KeyCode::Char(c) => app.input.push(c),
                            _ => {}
                        }
                    }
                    Some(Err(_)) | None => break,
                    _ => {}
                }
            }
            Some(msg) = rx.recv() => app.apply(msg),
            _ = ticker.tick(), if app.busy => { app.spinner = app.spinner.wrapping_add(1); }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.db != ":memory:" && !std::path::Path::new(&args.db).exists() {
        seed_sample_db(&args.db)?;
    }

    let runner: Arc<dyn SqlRunner> = Arc::new(SqliteRunner::new(args.db.clone()));
    let memory: Arc<dyn AgentMemory> = Arc::new(InMemoryAgentMemory::new());
    let llm = build_llm(args.llm, args.model.clone(), args.base_url.clone())?;
    let resolver: Arc<dyn UserResolver> = Arc::new(StaticUserResolver::admin());
    let mut registry = ToolRegistry::new();
    registry.register(RunSqlTool::new(runner.clone()));
    // Inject the live DB schema so questions about any table — including CSVs
    // loaded via /load — can be turned into SQL.
    let enhancer: Arc<dyn LlmContextEnhancer> = Arc::new(SchemaContextEnhancer::new(runner));
    let agent = Arc::new(
        AgentBuilder::new(llm, registry, resolver, memory)
            .llm_context_enhancer(enhancer)
            .build(),
    );

    let terminal = ratatui::init();
    let result = run(terminal, App::new(agent, args.db.clone())).await;
    ratatui::restore();
    result
}

#[cfg(test)]
mod wiring_tests {
    use super::*;
    use gauss_engine::traits::LlmService;
    use gauss_llm::MockLlmService;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_db() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "gauss_chat_tui_{}_{}.db",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().into_owned();
        seed_sample_db(&p).unwrap();
        p
    }

    /// End-to-end TUI backend wiring: a question flows through the real agent,
    /// each streamed component is serialized and rendered, and the rendered
    /// terminal lines contain the queried data plus a status update.
    #[tokio::test]
    async fn agent_stream_renders_to_terminal_lines() {
        let runner: Arc<dyn SqlRunner> = Arc::new(SqliteRunner::new(temp_db()));
        let memory: Arc<dyn AgentMemory> = Arc::new(InMemoryAgentMemory::new());
        let llm: Arc<dyn LlmService> = Arc::new(MockLlmService::new());
        let resolver: Arc<dyn UserResolver> = Arc::new(StaticUserResolver::admin());
        let mut reg = ToolRegistry::new();
        reg.register(RunSqlTool::new(runner));
        let agent = Arc::new(AgentBuilder::new(llm, reg, resolver, memory).build());

        let mut stream = Box::pin(agent.send_message(
            RequestContext::default(),
            "what tables exist?".to_string(),
            Some("t".to_string()),
        ));

        let mut text = String::new();
        let mut got_lines = false;
        let mut got_status = false;
        while let Some(c) = stream.next().await {
            let rich = c.rich_component.serialize_for_frontend();
            let simple = c
                .simple_component
                .as_ref()
                .map(gauss_engine::SimpleComponent::serialize_for_frontend);
            match render_component(&rich, simple.as_ref()) {
                Rendered::Lines(ls) => {
                    got_lines = true;
                    for l in ls {
                        for s in l.spans {
                            text.push_str(&s.content);
                            text.push('\n');
                        }
                    }
                }
                Rendered::Status(_) => got_status = true,
                Rendered::Skip => {}
            }
        }

        assert!(got_lines, "expected rendered transcript lines");
        assert!(got_status, "expected a status-bar update");
        // The seeded tables show up in the rendered dataframe.
        assert!(text.contains("customers"), "rendered output:\n{text}");
    }

    fn line_text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn viewport_follows_newest_by_default() {
        let lines: Vec<Line> = (0..10).map(|i| Line::from(format!("line {i}"))).collect();
        let v = viewport(&lines, 80, 3, 0);
        assert_eq!(v.lines.len(), 3);
        assert_eq!(line_text(&v.lines[2]), "line 9");
        assert_eq!(v.max_scroll, 7);
        assert_eq!(v.scroll, 0);
    }

    #[test]
    fn viewport_scrolls_back_into_history() {
        let lines: Vec<Line> = (0..10).map(|i| Line::from(format!("line {i}"))).collect();
        // Scroll up 5 lines from the bottom: the window ends at line index 5.
        let v = viewport(&lines, 80, 3, 5);
        assert_eq!(v.lines.len(), 3);
        assert_eq!(line_text(&v.lines[0]), "line 2");
        assert_eq!(line_text(&v.lines[2]), "line 4");
    }

    #[test]
    fn viewport_clamps_overscroll_to_top() {
        let lines: Vec<Line> = (0..10).map(|i| Line::from(format!("line {i}"))).collect();
        let v = viewport(&lines, 80, 3, 999);
        assert_eq!(v.scroll, 7, "scroll clamped to max_scroll");
        assert_eq!(
            line_text(&v.lines[0]),
            "line 0",
            "oldest line visible at top"
        );
    }
}
