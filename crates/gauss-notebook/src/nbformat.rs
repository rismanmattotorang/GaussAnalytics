//! Jupyter `.ipynb` (nbformat v4) interop.
//!
//! Import/export between a GaussAnalytics [`Notebook`] and the Jupyter notebook
//! format, so users can move work in and out of the broader ecosystem. Markdown
//! and Python cells map directly. GaussAnalytics-specific cells (SQL, NL2SQL,
//! Input, Chart, Big Number) round-trip via a leading **marker comment**
//! (`#%gauss kind=… db=… var=…`) on a code cell, so the file stays a valid
//! `.ipynb` that opens in Jupyter while preserving GaussAnalytics semantics.
//!
//! Pure (no I/O), so the round-trip is unit-tested without a kernel.

use gauss_core::domain::{CellKind, Notebook, NotebookCell};
use gauss_core::error::{CoreError, CoreResult};
use serde_json::{json, Value};
use uuid::Uuid;

/// The marker prefix that tags a non-Python GaussAnalytics cell in `.ipynb`.
const MARKER: &str = "#%gauss";

/// The marker wire name for a cell kind. Internal to this module (paired with
/// [`kind_from_str`]); not necessarily identical to the serde rename of the enum.
fn kind_str(kind: CellKind) -> &'static str {
    match kind {
        CellKind::Markdown => "markdown",
        CellKind::Python => "python",
        CellKind::Sql => "sql",
        CellKind::Nl2sql => "nl2sql",
        CellKind::Input => "input",
        CellKind::Chart => "chart",
        CellKind::BigNumber => "bignumber",
    }
}

fn kind_from_str(s: &str) -> Option<CellKind> {
    Some(match s {
        "markdown" => CellKind::Markdown,
        "python" => CellKind::Python,
        "sql" => CellKind::Sql,
        "nl2sql" => CellKind::Nl2sql,
        "input" => CellKind::Input,
        "chart" => CellKind::Chart,
        "bignumber" => CellKind::BigNumber,
        _ => return None,
    })
}

/// Split a cell body into nbformat `source` lines (each line keeps its trailing
/// newline except the last), the canonical Jupyter representation.
fn to_source_lines(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = s.split('\n').map(|l| format!("{l}\n")).collect();
    if let Some(last) = out.last_mut() {
        last.pop(); // drop the trailing newline on the final line
    }
    out
}

/// Read an nbformat `source` (a string or an array of strings) into one string.
fn read_source(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(lines) => lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// The marker line for a non-Python GaussAnalytics cell (`db`/`var` optional).
fn marker_line(cell: &NotebookCell) -> String {
    let mut s = format!("{MARKER} kind={}", kind_str(cell.kind));
    if let Some(db) = &cell.database_id {
        s.push_str(&format!(" db={db}"));
    }
    if let Some(var) = cell.output_var.as_ref().or(cell.input_var.as_ref()) {
        if !var.is_empty() {
            s.push_str(&format!(" var={var}"));
        }
    }
    s
}

/// Parse a marker line into `(kind, db, var)`; `None` if it isn't a marker.
fn parse_marker(line: &str) -> Option<(CellKind, Option<Uuid>, Option<String>)> {
    let rest = line.trim().strip_prefix(MARKER)?;
    let mut kind = None;
    let mut db = None;
    let mut var = None;
    for tok in rest.split_whitespace() {
        if let Some((k, v)) = tok.split_once('=') {
            match k {
                "kind" => kind = kind_from_str(v),
                "db" => db = Uuid::parse_str(v).ok(),
                "var" => var = Some(v.to_string()),
                _ => {}
            }
        }
    }
    kind.map(|k| (k, db, var))
}

/// Build one nbformat cell from a notebook cell.
fn cell_to_ipynb(cell: &NotebookCell) -> Value {
    if cell.kind == CellKind::Markdown {
        return json!({
            "cell_type": "markdown",
            "metadata": {},
            "source": to_source_lines(&cell.source),
        });
    }
    // Code cell. Non-Python kinds carry a leading marker so they round-trip.
    let mut source = Vec::new();
    if cell.kind != CellKind::Python {
        let mut marker = marker_line(cell);
        marker.push('\n');
        source.push(marker);
    }
    source.extend(to_source_lines(&cell.source));
    json!({
        "cell_type": "code",
        "metadata": {},
        "execution_count": Value::Null,
        "outputs": [],
        "source": source,
    })
}

/// Export a notebook to an nbformat v4 document.
pub fn export_ipynb(notebook: &Notebook) -> Value {
    let cells: Vec<Value> = notebook.cells.iter().map(cell_to_ipynb).collect();
    json!({
        "cells": cells,
        "metadata": {
            "kernelspec": { "display_name": "Python 3", "language": "python", "name": "python3" },
            "language_info": { "name": "python" },
            "gauss": { "name": notebook.name },
        },
        "nbformat": 4,
        "nbformat_minor": 5,
    })
}

/// Parse the cells of an nbformat document into GaussAnalytics cells. Unknown
/// cell types are skipped; code cells without a marker become Python cells.
pub fn import_cells(doc: &Value) -> CoreResult<Vec<NotebookCell>> {
    let cells = doc
        .get("cells")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::InvalidQuery("not an .ipynb: missing `cells`".into()))?;

    let mut out = Vec::with_capacity(cells.len());
    for c in cells {
        let cell_type = c.get("cell_type").and_then(Value::as_str).unwrap_or("");
        let source = read_source(c.get("source").unwrap_or(&Value::Null));
        match cell_type {
            "markdown" => out.push(NotebookCell {
                id: Uuid::new_v4(),
                kind: CellKind::Markdown,
                source,
                database_id: None,
                output_var: None,
                input_var: None,
            }),
            "code" => out.push(import_code_cell(&source)),
            // Skip raw/unknown cell types.
            _ => {}
        }
    }
    Ok(out)
}

/// Assign a marker's `var` to the right field for the cell kind: `output_var`
/// for SQL/NL2SQL (the produced DataFrame), `input_var` for Input/Chart/Big
/// Number (the consumed variable).
fn vars_for(kind: CellKind, var: Option<String>) -> (Option<String>, Option<String>) {
    match kind {
        CellKind::Sql | CellKind::Nl2sql => (var, None),
        CellKind::Input | CellKind::Chart | CellKind::BigNumber => (None, var),
        _ => (None, None),
    }
}

/// Map a code cell's source to a notebook cell, honoring a `#%gauss` marker. A
/// marker on the first line restores a GaussAnalytics cell (with the body after
/// it); anything else is a plain Python cell.
fn import_code_cell(source: &str) -> NotebookCell {
    let id = Uuid::new_v4();
    // The body is everything after the first line (empty for a marker-only cell).
    let (first, body) = match source.split_once('\n') {
        Some((first, rest)) => (first, rest.to_string()),
        None => (source, String::new()),
    };
    if let Some((kind, db, var)) = parse_marker(first) {
        let (output_var, input_var) = vars_for(kind, var);
        return NotebookCell {
            id,
            kind,
            source: body,
            database_id: db,
            output_var,
            input_var,
        };
    }
    NotebookCell {
        id,
        kind: CellKind::Python,
        source: source.to_string(),
        database_id: None,
        output_var: None,
        input_var: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn cell(
        kind: CellKind,
        source: &str,
        db: Option<Uuid>,
        ov: Option<&str>,
        iv: Option<&str>,
    ) -> NotebookCell {
        NotebookCell {
            id: Uuid::new_v4(),
            kind,
            source: source.into(),
            database_id: db,
            output_var: ov.map(String::from),
            input_var: iv.map(String::from),
        }
    }

    fn notebook(cells: Vec<NotebookCell>) -> Notebook {
        Notebook {
            id: Uuid::new_v4(),
            name: "Demo".into(),
            collection_id: None,
            cells,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn round_trips_all_cell_kinds() {
        let db = Uuid::new_v4();
        let nb = notebook(vec![
            cell(CellKind::Markdown, "# Title\nsome notes", None, None, None),
            cell(CellKind::Python, "x = 1\nprint(x)", None, None, None),
            cell(
                CellKind::Sql,
                "select 1 as n",
                Some(db),
                Some("orders"),
                None,
            ),
            cell(
                CellKind::Nl2sql,
                "top customers",
                Some(db),
                Some("cust"),
                None,
            ),
            cell(CellKind::Input, "10", None, None, Some("threshold")),
            cell(CellKind::Chart, "", None, None, Some("orders")),
            cell(CellKind::BigNumber, "", None, None, Some("orders")),
        ]);

        let doc = export_ipynb(&nb);
        assert_eq!(doc["nbformat"], 4);
        assert_eq!(doc["metadata"]["gauss"]["name"], "Demo");

        let back = import_cells(&doc).unwrap();
        assert_eq!(back.len(), nb.cells.len());
        for (orig, got) in nb.cells.iter().zip(back.iter()) {
            assert_eq!(orig.kind, got.kind, "kind mismatch");
            assert_eq!(
                orig.source, got.source,
                "source mismatch for {:?}",
                orig.kind
            );
            assert_eq!(orig.database_id, got.database_id, "db mismatch");
            assert_eq!(orig.output_var, got.output_var, "output_var mismatch");
            assert_eq!(orig.input_var, got.input_var, "input_var mismatch");
        }
    }

    #[test]
    fn imports_a_plain_jupyter_notebook() {
        // A vanilla .ipynb with no gauss markers becomes markdown + python cells.
        let doc = json!({
            "cells": [
                { "cell_type": "markdown", "source": ["# Hello\n", "world"] },
                { "cell_type": "code", "source": "import pandas as pd\ndf = pd.DataFrame()", "outputs": [] },
                { "cell_type": "raw", "source": ["ignored"] }
            ],
            "nbformat": 4, "nbformat_minor": 5, "metadata": {}
        });
        let cells = import_cells(&doc).unwrap();
        assert_eq!(cells.len(), 2); // raw cell skipped
        assert_eq!(cells[0].kind, CellKind::Markdown);
        assert_eq!(cells[0].source, "# Hello\nworld");
        assert_eq!(cells[1].kind, CellKind::Python);
        assert!(cells[1].source.contains("import pandas"));
    }

    #[test]
    fn rejects_non_ipynb() {
        assert!(import_cells(&json!({ "foo": 1 })).is_err());
    }
}
