//! `gauss-tui` — the GaussAnalytics operator administration console.
//!
//! A fast, keyboard-driven terminal UI (built on Ratatui) for the people who
//! run GaussAnalytics: inspect health, data sources, users/sessions, background
//! jobs, and the MCP/AI audit trail. It speaks the same HTTP API as the web UI,
//! so it is a first-class client rather than a privileged backdoor.
//!
//! It reads live data from the server (`GAUSS_API_URL`, default
//! `http://127.0.0.1:3000`); the admin Users view needs `GAUSS_API_TOKEN`.
//!
//! Navigation: `←/→` or `Tab` to switch · `1`–`6` to jump · `r` to refresh ·
//! `q` to quit.

#![forbid(unsafe_code)]

pub mod app;
pub mod client;

use std::io;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Tabs};
use ratatui::{DefaultTerminal, Frame};

pub use app::{App, TABS};
pub use client::ApiClient;

/// Launch the administration console, restoring the terminal on exit.
pub fn run() -> io::Result<()> {
    let client = ApiClient::from_env();
    let mut app = App::new();
    app.has_token = client.has_token();
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &client);
    ratatui::restore();
    result
}

/// The draw/poll loop. Polls with a timeout so the UI stays responsive.
fn event_loop(terminal: &mut DefaultTerminal, app: &mut App, client: &ApiClient) -> io::Result<()> {
    while !app.should_quit {
        if app.should_refresh {
            refresh(app, client);
            app.should_refresh = false;
        }
        terminal.draw(|frame| draw(frame, app))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code);
                }
            }
        }
    }
    Ok(())
}

/// Fetch live data from the server into `app`, collecting any per-section
/// errors for display rather than aborting.
fn refresh(app: &mut App, client: &ApiClient) {
    app.errors.clear();
    match client.health() {
        Ok(h) => app.health = Some(format!("{} v{}", h.status, h.version)),
        Err(e) => {
            app.health = None;
            app.errors.push(format!("health: {e}"));
        }
    }
    match client.databases() {
        Ok(d) => app.databases = d,
        Err(e) => app.errors.push(format!("databases: {e}")),
    }
    if let Ok(c) = client.cards() {
        app.cards = c.len();
    }
    if let Ok(d) = client.dashboards() {
        app.dashboards = d.len();
    }
    if client.has_token() {
        match client.users() {
            Ok(u) => app.users = u,
            Err(e) => app.errors.push(format!("users: {e}")),
        }
    }
}

/// The accent used throughout the console (matches the web UI's cyan).
const ACCENT: Color = Color::Rgb(56, 189, 248);
const MUTED: Color = Color::Rgb(138, 147, 171);

/// Render one frame. Public so the buffer can be rendered headlessly (e.g. for
/// documentation screenshots) without driving a real terminal.
pub fn draw(frame: &mut Frame, app: &App) {
    let areas = Layout::vertical([
        Constraint::Length(3), // tab bar
        Constraint::Min(0),    // body
        Constraint::Length(1), // footer
    ])
    .split(frame.area());

    let tabs = Tabs::new(TABS.iter().copied().map(Line::from).collect::<Vec<_>>())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT))
                .title(Span::styled(
                    " GaussAnalytics — Admin Console ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                )),
        )
        .divider(Span::styled("·", Style::default().fg(MUTED)))
        .select(app.selected)
        .style(Style::default().fg(MUTED))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, areas[0]);

    let body = Paragraph::new(app.body_text()).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(41, 49, 63)))
            .title(Span::styled(
                format!(" {} ", app.active_tab()),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )),
    );
    frame.render_widget(body, areas[1]);

    frame.render_widget(Paragraph::new(footer_line()), areas[2]);
}

/// The footer key hints, with keys accented and labels dimmed.
fn footer_line() -> Line<'static> {
    let key = |k: &'static str| Span::styled(k, Style::default().fg(ACCENT));
    let lbl = |t: &'static str| Span::styled(t, Style::default().fg(MUTED));
    Line::from(vec![
        lbl(" "),
        key("←/→"),
        lbl(" or "),
        key("Tab"),
        lbl(" switch · "),
        key("1-6"),
        lbl(" jump · "),
        key("r"),
        lbl(" refresh · "),
        key("q"),
        lbl(" quit "),
    ])
}
