//! Pure rendering of agent UI components (the SSE wire JSON) into styled
//! terminal lines. Kept separate from the event loop so it can be unit-tested.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

/// Gaussian Technologies brand palette, tuned for legibility on dark terminals.
/// `BRAND` is the lighter cobalt (logo blue lifted for contrast); `BRAND_DEEP`
/// is the logo blue itself.
pub const BRAND: Color = Color::Rgb(76, 127, 224); // #4C7FE0
pub const BRAND_DEEP: Color = Color::Rgb(43, 91, 181); // #2B5BB5

/// Outcome of rendering one component.
pub enum Rendered {
    /// Append these lines to the transcript.
    Lines(Vec<Line<'static>>),
    /// Update the status bar text.
    Status(String),
    /// Nothing to show (e.g. chat-input updates).
    Skip,
}

fn dim(s: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(s.into(), Style::default().fg(Color::DarkGray)))
}

fn plain(s: impl Into<String>) -> Line<'static> {
    Line::from(s.into())
}

fn styled(s: impl Into<String>, style: Style) -> Line<'static> {
    Line::from(Span::styled(s.into(), style))
}

fn str_field<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Render a component given its `rich` JSON and optional `simple` fallback.
pub fn render_component(rich: &Value, simple: Option<&Value>) -> Rendered {
    let ty = rich.get("type").and_then(Value::as_str).unwrap_or("");
    let data = rich.get("data").cloned().unwrap_or(Value::Null);

    match ty {
        "text" | "notification" => {
            let content = if ty == "text" {
                str_field(&data, "content")
            } else {
                str_field(&data, "message")
            };
            Rendered::Lines(text_block("", content))
        }
        "card" => {
            let title = str_field(&data, "title");
            Rendered::Lines(text_block(title, str_field(&data, "content")))
        }
        "code_block" => {
            let mut lines = vec![styled(
                "┌─ code ─".to_string(),
                Style::default().fg(BRAND_DEEP),
            )];
            for l in str_field(&data, "content").lines() {
                lines.push(styled(format!("│ {l}"), Style::default().fg(BRAND)));
            }
            Rendered::Lines(lines)
        }
        "status_card" => {
            let status = str_field(&data, "status");
            let color = status_color(status);
            let title = format!("{} {}", str_field(&data, "icon"), str_field(&data, "title"));
            let mut lines = vec![styled(
                title.trim().to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )];
            let desc = str_field(&data, "description");
            if !desc.is_empty() {
                lines.push(dim(format!("  {desc}")));
            }
            Rendered::Lines(lines)
        }
        "dataframe" => Rendered::Lines(render_dataframe(&data)),
        "chart" => Rendered::Lines(render_chart(&data)),
        "status_bar_update" => {
            let msg = str_field(&data, "message");
            let detail = str_field(&data, "detail");
            let text = if detail.is_empty() {
                msg.to_string()
            } else {
                format!("{msg} — {detail}")
            };
            Rendered::Status(text)
        }
        "task_tracker_update" => {
            if data.get("operation").and_then(Value::as_str) == Some("add_task") {
                if let Some(title) = data
                    .get("task")
                    .and_then(|t| t.get("title"))
                    .and_then(Value::as_str)
                {
                    return Rendered::Lines(vec![dim(format!("  • {title}"))]);
                }
            }
            Rendered::Skip
        }
        "chat_input_update" => Rendered::Skip,
        "badge" => Rendered::Lines(vec![plain(format!("[{}]", str_field(&data, "text")))]),
        "icon_text" => Rendered::Lines(vec![plain(format!(
            "{} {}",
            str_field(&data, "icon"),
            str_field(&data, "text")
        ))]),
        _ => {
            // Fall back to the simple component's text if present.
            if let Some(t) = simple.and_then(|s| s.get("text")).and_then(Value::as_str) {
                Rendered::Lines(text_block("", t))
            } else {
                Rendered::Skip
            }
        }
    }
}

fn status_color(status: &str) -> Color {
    match status {
        "success" | "completed" | "idle" => Color::Green,
        "error" | "failed" => Color::Red,
        "warning" => Color::Yellow,
        _ => BRAND,
    }
}

/// An assistant block: optional bold title + content lines, prefixed once.
fn text_block(title: &str, content: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !title.is_empty() {
        lines.push(styled(
            title.to_string(),
            Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
        ));
    }
    let prefix_style = Style::default().fg(Color::White);
    for (i, l) in content.lines().enumerate() {
        let text = if i == 0 && title.is_empty() {
            format!("⏵ {l}")
        } else {
            format!("  {l}")
        };
        lines.push(styled(text, prefix_style));
    }
    if lines.is_empty() {
        lines.push(plain(""));
    }
    lines
}

const MAX_COL_WIDTH: usize = 24;
const MAX_DF_ROWS: usize = 50;

fn cell_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() > w {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    } else {
        s.to_string()
    }
}

fn render_dataframe(data: &Value) -> Vec<Line<'static>> {
    let cols: Vec<String> = data
        .get("columns")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(cell_str).collect())
        .unwrap_or_default();
    let rows: Vec<&Value> = data
        .get("rows")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();

    if cols.is_empty() {
        return vec![dim("(empty result)")];
    }

    // Compute per-column widths (header + sampled cells), capped.
    let mut widths: Vec<usize> = cols.iter().map(|c| c.chars().count()).collect();
    for r in rows.iter().take(MAX_DF_ROWS) {
        for (i, c) in cols.iter().enumerate() {
            let len = r.get(c).map_or(0, |v| cell_str(v).chars().count());
            if len > widths[i] {
                widths[i] = len;
            }
        }
    }
    for w in &mut widths {
        *w = (*w).min(MAX_COL_WIDTH);
    }

    let fmt_row = |fields: &[String]| -> String {
        fields
            .iter()
            .enumerate()
            .map(|(i, f)| format!("{:<width$}", truncate(f, widths[i]), width = widths[i]))
            .collect::<Vec<_>>()
            .join(" │ ")
    };

    let header = fmt_row(&cols);
    let sep: String = header
        .chars()
        .map(|c| if c == '│' { '┼' } else { '─' })
        .collect();

    let mut lines = vec![
        styled(
            header,
            Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
        ),
        dim(sep),
    ];
    for r in rows.iter().take(MAX_DF_ROWS) {
        let fields: Vec<String> = cols
            .iter()
            .map(|c| r.get(c).map(cell_str).unwrap_or_default())
            .collect();
        lines.push(plain(fmt_row(&fields)));
    }
    let total = data
        .get("row_count")
        .and_then(Value::as_u64)
        .map_or(rows.len(), |n| n as usize);
    if total > MAX_DF_ROWS {
        lines.push(dim(format!("… {MAX_DF_ROWS} of {total} rows shown")));
    } else {
        lines.push(dim(format!("{total} row(s)")));
    }
    lines
}

fn render_chart(data: &Value) -> Vec<Line<'static>> {
    let title = data.get("title").and_then(Value::as_str).unwrap_or("");
    let ctype = str_field(data, "chart_type");
    let mut lines = vec![styled(
        format!("📊 {}", if title.is_empty() { ctype } else { title }),
        Style::default().fg(BRAND).add_modifier(Modifier::BOLD),
    )];

    let series = data.pointer("/data/data/0").cloned().unwrap_or(Value::Null);
    let xs = series
        .get("x")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let ys = series
        .get("y")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let nums: Vec<f64> = ys.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
    let max = nums.iter().cloned().fold(0.0_f64, f64::max).max(1.0);
    for i in 0..xs.len().min(nums.len()).min(20) {
        let bar = "█".repeat(((nums[i] / max) * 30.0).round() as usize);
        lines.push(plain(format!(
            "  {:<16} {} {}",
            truncate(&cell_str(&xs[i]), 16),
            bar,
            nums[i]
        )));
    }
    if xs.is_empty() {
        lines.push(dim("  (no plottable series)"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line_text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn renders_text_block() {
        let rich = json!({"type":"text","data":{"content":"Hello\nworld","markdown":true}});
        match render_component(&rich, None) {
            Rendered::Lines(ls) => {
                let joined = ls.iter().map(line_text).collect::<Vec<_>>().join("\n");
                assert!(joined.contains("Hello"));
                assert!(joined.contains("world"));
            }
            _ => panic!("expected lines"),
        }
    }

    #[test]
    fn status_bar_becomes_status() {
        let rich = json!({"type":"status_bar_update","data":{"status":"working","message":"Processing","detail":"step 1"}});
        match render_component(&rich, None) {
            Rendered::Status(s) => assert_eq!(s, "Processing — step 1"),
            _ => panic!("expected status"),
        }
    }

    #[test]
    fn dataframe_has_header_and_rows() {
        let rich = json!({"type":"dataframe","data":{
            "columns":["name","value"],
            "rows":[{"name":"Acme","value":10},{"name":"Globex","value":20}],
            "row_count":2
        }});
        match render_component(&rich, None) {
            Rendered::Lines(ls) => {
                let joined = ls.iter().map(line_text).collect::<Vec<_>>().join("\n");
                assert!(joined.contains("name"));
                assert!(joined.contains("Acme"));
                assert!(joined.contains("Globex"));
                assert!(joined.contains("2 row"));
            }
            _ => panic!("expected lines"),
        }
    }

    #[test]
    fn chat_input_update_is_skipped() {
        let rich = json!({"type":"chat_input_update","data":{"placeholder":"x","disabled":false}});
        assert!(matches!(render_component(&rich, None), Rendered::Skip));
    }

    #[test]
    fn unknown_falls_back_to_simple_text() {
        let rich = json!({"type":"mystery","data":{}});
        let simple = json!({"type":"text","text":"fallback"});
        match render_component(&rich, Some(&simple)) {
            Rendered::Lines(ls) => {
                assert!(ls.iter().map(line_text).any(|t| t.contains("fallback")));
            }
            _ => panic!("expected lines"),
        }
    }
}
