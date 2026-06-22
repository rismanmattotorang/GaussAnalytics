//! TUI application state and (terminal-independent) update logic.
//!
//! State transitions live here, separate from the render/event plumbing and the
//! HTTP client, so they are unit-testable without a real terminal or server.

use ratatui::crossterm::event::KeyCode;

use crate::client::{DbRow, UserRow};

/// The administration views available in the console.
pub const TABS: &[&str] = &["Overview", "Databases", "Users", "Jobs", "MCP & AI", "Logs"];

/// The mutable state of the TUI.
#[derive(Default)]
pub struct App {
    /// Index into [`TABS`] of the active view.
    pub selected: usize,
    /// Set when the operator asks to quit.
    pub should_quit: bool,
    /// Set when the operator requests a data refresh (consumed by the loop).
    pub should_refresh: bool,
    /// Health line (`status vX.Y`), if the last fetch succeeded.
    pub health: Option<String>,
    /// Connected data sources from the last fetch.
    pub databases: Vec<DbRow>,
    /// Users from the last fetch (requires an admin token).
    pub users: Vec<UserRow>,
    /// Whether an admin token is configured (affects the Users view hint).
    pub has_token: bool,
    /// Saved-question count from the last fetch.
    pub cards: usize,
    /// Dashboard count from the last fetch.
    pub dashboards: usize,
    /// Non-fatal errors from the last refresh, shown to the operator.
    pub errors: Vec<String>,
}

impl App {
    pub fn new() -> Self {
        // Request an initial refresh as soon as the loop starts.
        Self {
            should_refresh: true,
            ..Self::default()
        }
    }

    /// The title of the active tab.
    pub fn active_tab(&self) -> &'static str {
        TABS[self.selected]
    }

    /// Advance to the next tab (wrapping).
    pub fn next_tab(&mut self) {
        self.selected = (self.selected + 1) % TABS.len();
    }

    /// Move to the previous tab (wrapping).
    pub fn prev_tab(&mut self) {
        self.selected = (self.selected + TABS.len() - 1) % TABS.len();
    }

    /// Apply a key press to the state.
    pub fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('r') => self.should_refresh = true,
            KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => self.next_tab(),
            KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => self.prev_tab(),
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let idx = c.to_digit(10).unwrap() as usize;
                if (1..=TABS.len()).contains(&idx) {
                    self.selected = idx - 1;
                }
            }
            _ => {}
        }
    }

    /// The rendered body text for the active view, built from live data.
    pub fn body_text(&self) -> String {
        match self.active_tab() {
            "Overview" => {
                let mut s = String::new();
                let row = |label: &str, val: String| format!("{label:<12}{val}\n");
                s.push_str(&row(
                    "Status:",
                    self.health
                        .as_deref()
                        .unwrap_or("(unreachable)")
                        .to_string(),
                ));
                s.push_str(&row("Databases:", self.databases.len().to_string()));
                s.push_str(&row("Users:", self.users.len().to_string()));
                s.push_str(&row("Cards:", self.cards.to_string()));
                s.push_str(&row("Dashboards:", self.dashboards.to_string()));
                if !self.errors.is_empty() {
                    s.push_str("\nIssues:\n");
                    for e in &self.errors {
                        s.push_str(&format!("  • {e}\n"));
                    }
                }
                s
            }
            "Databases" => {
                if self.databases.is_empty() {
                    "No data sources (or server unreachable). Press 'r' to refresh.".to_string()
                } else {
                    let mut s = String::from("NAME                 KIND        SYNCED\n");
                    for d in &self.databases {
                        s.push_str(&format!(
                            "{:<20} {:<11} {}\n",
                            d.name,
                            d.kind,
                            if d.is_synced { "yes" } else { "no" }
                        ));
                    }
                    s
                }
            }
            "Users" => {
                if !self.has_token {
                    "Users require an admin token. Set GAUSS_API_TOKEN and press 'r'.".to_string()
                } else if self.users.is_empty() {
                    "No users (or unauthorized). Press 'r' to refresh.".to_string()
                } else {
                    let mut s =
                        String::from("EMAIL                          NAME                 ADMIN\n");
                    for u in &self.users {
                        s.push_str(&format!(
                            "{:<30} {:<20} {}\n",
                            u.email,
                            u.display_name,
                            if u.is_admin { "yes" } else { "no" }
                        ));
                    }
                    s
                }
            }
            "Jobs" => "Background jobs: schema sync, refreshes, and alerts (Phase 3).".to_string(),
            "MCP & AI" => "Gaussian MCP server registry, tool allow-lists, and NL2SQL status.\n\
                 Review the audit trail of agentic tool calls."
                .to_string(),
            "Logs" => "Recent structured logs and request traces.".to_string(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_navigation_wraps() {
        let mut app = App::new();
        app.prev_tab();
        assert_eq!(app.selected, TABS.len() - 1);
        app.next_tab();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn digit_jumps_to_tab() {
        let mut app = App::new();
        app.handle_key(KeyCode::Char('3'));
        assert_eq!(app.active_tab(), TABS[2]);
    }

    #[test]
    fn q_quits_and_r_requests_refresh() {
        let mut app = App::new();
        app.should_refresh = false;
        app.handle_key(KeyCode::Char('r'));
        assert!(app.should_refresh);
        app.handle_key(KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn databases_view_renders_rows() {
        let mut app = App::new();
        app.selected = 1; // Databases
        app.databases = vec![DbRow {
            name: "sales".into(),
            kind: "sqlite".into(),
            is_synced: true,
        }];
        let body = app.body_text();
        assert!(body.contains("sales"));
        assert!(body.contains("sqlite"));
    }
}
