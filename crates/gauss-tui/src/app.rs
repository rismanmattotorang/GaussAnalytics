//! TUI application state and (terminal-independent) update logic.
//!
//! Keeping state transitions out of the render/event plumbing makes them
//! unit-testable without a real terminal.

use ratatui::crossterm::event::KeyCode;

/// The administration views available in the console.
pub const TABS: &[&str] = &["Overview", "Databases", "Users", "Jobs", "MCP & AI", "Logs"];

/// The mutable state of the TUI.
#[derive(Default)]
pub struct App {
    /// Index into [`TABS`] of the active view.
    pub selected: usize,
    /// Set when the operator asks to quit.
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self::default()
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

    /// Placeholder body text describing what the active view will surface.
    ///
    /// These views become live data (over the server's HTTP API) as the
    /// corresponding backend subsystems land in later phases.
    pub fn body_text(&self) -> &'static str {
        match self.active_tab() {
            "Overview" => {
                "System health, version, and at-a-glance counts.\n\
                 Live data is sourced from GET /api/health and /api/version."
            }
            "Databases" => {
                "Connected data sources, sync status, and table counts.\n\
                 Manage connections and trigger schema sync (Phase 2)."
            }
            "Users" => {
                "Users, roles, and active sessions.\n\
                 Invite, deactivate, and revoke sessions (Phase 2)."
            }
            "Jobs" => {
                "Background jobs: schema sync, refreshes, and alerts.\n\
                 Inspect, retry, and pause the scheduler (Phase 3)."
            }
            "MCP & AI" => {
                "Gaussian MCP server registry, tool allow-lists, and NL2SQL\n\
                 status. Review the audit trail of agentic tool calls."
            }
            "Logs" => "Recent structured logs and request traces.",
            _ => "",
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
    fn q_quits() {
        let mut app = App::new();
        assert!(!app.should_quit);
        app.handle_key(KeyCode::Char('q'));
        assert!(app.should_quit);
    }
}
