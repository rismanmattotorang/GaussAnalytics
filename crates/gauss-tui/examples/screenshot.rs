//! Render the admin console to an SVG (no real terminal), for documentation
//! screenshots. Draws the *actual* widget tree via ratatui's `TestBackend`, then
//! serializes the resulting cell buffer to an SVG monospace grid.
//!
//! Usage: `cargo run -p gauss-tui --example screenshot > console.svg`

use std::fmt::Write as _;

use gauss_tui::app::{App, TABS};
use gauss_tui::client::{DbRow, UserRow};
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

const COLS: u16 = 96;
const ROWS: u16 = 26;
const CW: f32 = 8.6; // character advance (px) at the chosen font size
const CH: f32 = 19.0; // line height (px)
const FONT_PX: f32 = 14.5;

fn main() {
    // A representative, populated console (the Overview view).
    let mut app = App::new();
    app.should_refresh = false;
    app.has_token = true;
    app.health = Some("ok v1.0.0".into());
    app.cards = 7;
    app.dashboards = 3;
    app.databases = vec![
        DbRow {
            name: "warehouse".into(),
            kind: "postgres".into(),
            is_synced: true,
        },
        DbRow {
            name: "events".into(),
            kind: "clickhouse".into(),
            is_synced: true,
        },
        DbRow {
            name: "billing".into(),
            kind: "mysql".into(),
            is_synced: true,
        },
        DbRow {
            name: "analytics".into(),
            kind: "snowflake".into(),
            is_synced: false,
        },
    ];
    app.users = vec![
        UserRow {
            email: "admin@gaussian.tech".into(),
            display_name: "Administrator".into(),
            is_admin: true,
        },
        UserRow {
            email: "ana@gaussian.tech".into(),
            display_name: "Ana Ngebe".into(),
            is_admin: false,
        },
    ];

    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).expect("test backend");
    terminal.draw(|f| gauss_tui::draw(f, &app)).expect("draw");

    print!("{}", buffer_to_svg(terminal.backend().buffer()));
    let _ = TABS; // keep the import meaningful if the view set changes
}

/// Serialize a ratatui cell buffer into a standalone SVG.
fn buffer_to_svg(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area;
    let w = area.width as f32 * CW;
    let h = area.height as f32 * CH;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" \
         viewBox=\"0 0 {w:.0} {h:.0}\" font-family=\"'DejaVu Sans Mono','Menlo',monospace\" \
         font-size=\"{FONT_PX}\">"
    );
    // Window background with a subtle rounded panel, matching the web UI.
    let _ = writeln!(
        s,
        "<rect x=\"0\" y=\"0\" width=\"{w:.0}\" height=\"{h:.0}\" rx=\"10\" fill=\"#0b1020\"/>"
    );

    // Cell backgrounds first, then glyphs on top.
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = buf.cell((x, y)).expect("cell");
            if let Some(bg) = color_hex(cell.bg) {
                let _ = writeln!(
                    s,
                    "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{bg}\"/>",
                    x as f32 * CW,
                    y as f32 * CH,
                    CW + 0.5,
                    CH
                );
            }
        }
    }
    for y in 0..area.height {
        for x in 0..area.width {
            let cell = buf.cell((x, y)).expect("cell");
            let sym = cell.symbol();
            if sym.trim().is_empty() {
                continue;
            }
            let fg = color_hex(cell.fg).unwrap_or_else(|| "#c8d3f5".into());
            let bold = cell.modifier.contains(Modifier::BOLD);
            let dim = cell.modifier.contains(Modifier::DIM);
            let _ = writeln!(
                s,
                "<text x=\"{:.2}\" y=\"{:.2}\" fill=\"{fg}\"{}{} xml:space=\"preserve\">{}</text>",
                x as f32 * CW + 1.0,
                y as f32 * CH + FONT_PX,
                if bold { " font-weight=\"bold\"" } else { "" },
                if dim { " opacity=\"0.6\"" } else { "" },
                escape(sym),
            );
        }
    }
    s.push_str("</svg>\n");
    s
}

/// Map a ratatui color to an SVG hex, or `None` for the terminal default.
fn color_hex(c: Color) -> Option<String> {
    Some(match c {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#0b1020".into(),
        Color::White | Color::Gray => "#e6e9f2".into(),
        Color::Cyan | Color::LightCyan => "#38bdf8".into(),
        Color::Green | Color::LightGreen => "#34d399".into(),
        Color::Red | Color::LightRed => "#f87171".into(),
        Color::Yellow | Color::LightYellow => "#fbbf24".into(),
        Color::Blue | Color::LightBlue => "#818cf8".into(),
        Color::Magenta | Color::LightMagenta => "#c084fc".into(),
        Color::DarkGray => "#8a93ab".into(),
        _ => "#c8d3f5".into(),
    })
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
