//! Ingest a CSV document into a SQLite table so it becomes queryable in natural
//! language. Headers become columns (sanitized to safe identifiers), column
//! types are inferred from the data (INTEGER → REAL → TEXT), and the rows are
//! bulk-inserted in a single transaction. Re-uploading the same table name
//! replaces it.

use csv::ReaderBuilder;
use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Connection};

/// A column of an ingested table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvColumn {
    pub name: String,
    /// The inferred SQLite type: `INTEGER`, `REAL`, or `TEXT`.
    pub sql_type: String,
}

/// The outcome of a successful ingest.
#[derive(Debug, Clone)]
pub struct CsvIngestSummary {
    pub table: String,
    pub columns: Vec<CsvColumn>,
    pub row_count: usize,
}

/// Turn an arbitrary header/name into a safe lowercase SQL identifier:
/// `[a-z0-9_]`, never empty, never leading-digit. Falls back to `fallback`.
fn sanitize_ident(raw: &str, fallback: &str) -> String {
    let mut s: String = raw
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    let s = s.trim_matches('_').to_string();
    let s = if s.is_empty() {
        fallback.to_string()
    } else {
        s
    };
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("t_{s}")
    } else {
        s
    }
}

/// Ingest `csv_data` into table `table_hint` of the SQLite database at
/// `db_path`. Returns the created table name (sanitized), its columns with
/// inferred types, and the row count. Errors are human-readable strings.
pub fn ingest_csv(
    db_path: &str,
    table_hint: &str,
    csv_data: &str,
) -> Result<CsvIngestSummary, String> {
    if db_path == ":memory:" {
        return Err("CSV upload requires a file-backed SQLite database (not :memory:)".into());
    }
    let table = sanitize_ident(table_hint, "uploaded_table");

    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(csv_data.as_bytes());

    let headers = rdr
        .headers()
        .map_err(|e| format!("could not read CSV header row: {e}"))?
        .clone();
    if headers.is_empty() {
        return Err("CSV has no header row".into());
    }

    // Unique, sanitized column names.
    let mut names: Vec<String> = Vec::with_capacity(headers.len());
    for (i, h) in headers.iter().enumerate() {
        let base = sanitize_ident(h, &format!("col_{}", i + 1));
        let mut name = base.clone();
        let mut n = 2;
        while names.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        names.push(name);
    }
    let ncols = names.len();

    // Materialize rows (padded/truncated to the header width).
    let mut records: Vec<Vec<String>> = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| format!("could not read CSV row: {e}"))?;
        records.push(
            (0..ncols)
                .map(|i| rec.get(i).unwrap_or("").to_string())
                .collect(),
        );
    }

    // Infer a type per column: INTEGER if every non-empty value parses as i64,
    // else REAL if every one parses as f64, else TEXT.
    let mut types: Vec<&'static str> = vec!["TEXT"; ncols];
    for (c, ty) in types.iter_mut().enumerate() {
        let mut seen = false;
        let mut all_int = true;
        let mut all_real = true;
        for row in &records {
            let v = row[c].trim();
            if v.is_empty() {
                continue;
            }
            seen = true;
            if v.parse::<i64>().is_err() {
                all_int = false;
            }
            if v.parse::<f64>().is_err() {
                all_real = false;
            }
        }
        *ty = if !seen {
            "TEXT"
        } else if all_int {
            "INTEGER"
        } else if all_real {
            "REAL"
        } else {
            "TEXT"
        };
    }

    let mut conn = Connection::open(db_path).map_err(|e| format!("open database: {e}"))?;
    let cols_ddl = names
        .iter()
        .zip(&types)
        .map(|(n, t)| format!("\"{n}\" {t}"))
        .collect::<Vec<_>>()
        .join(", ");

    let tx = conn
        .transaction()
        .map_err(|e| format!("begin transaction: {e}"))?;
    tx.execute_batch(&format!(
        "DROP TABLE IF EXISTS \"{table}\"; CREATE TABLE \"{table}\" ({cols_ddl});"
    ))
    .map_err(|e| format!("create table: {e}"))?;

    let placeholders = vec!["?"; ncols].join(", ");
    let insert_sql = format!("INSERT INTO \"{table}\" VALUES ({placeholders})");
    {
        let mut stmt = tx
            .prepare(&insert_sql)
            .map_err(|e| format!("prepare insert: {e}"))?;
        for row in &records {
            let params: Vec<SqlValue> = (0..ncols)
                .map(|c| {
                    let v = row[c].trim();
                    if v.is_empty() {
                        return SqlValue::Null;
                    }
                    match types[c] {
                        "INTEGER" => v.parse::<i64>().map_or(SqlValue::Null, SqlValue::Integer),
                        "REAL" => v.parse::<f64>().map_or(SqlValue::Null, SqlValue::Real),
                        _ => SqlValue::Text(row[c].clone()),
                    }
                })
                .collect();
            stmt.execute(params_from_iter(params.iter()))
                .map_err(|e| format!("insert row: {e}"))?;
        }
    }
    tx.commit().map_err(|e| format!("commit: {e}"))?;

    Ok(CsvIngestSummary {
        table,
        columns: names
            .into_iter()
            .zip(types)
            .map(|(name, t)| CsvColumn {
                name,
                sql_type: t.to_string(),
            })
            .collect(),
        row_count: records.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_db() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pt_ingest_{}_{}.db",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn ingests_with_type_inference() {
        let db = temp_db();
        let csv = "Name,Age,Score\nAlice,30,9.5\nBob,25,8\n";
        let s = ingest_csv(&db, "People List.csv", csv).unwrap();

        assert_eq!(s.table, "people_list_csv");
        assert_eq!(s.row_count, 2);
        assert_eq!(
            s.columns,
            vec![
                CsvColumn {
                    name: "name".into(),
                    sql_type: "TEXT".into()
                },
                CsvColumn {
                    name: "age".into(),
                    sql_type: "INTEGER".into()
                },
                CsvColumn {
                    name: "score".into(),
                    sql_type: "REAL".into()
                },
            ]
        );

        // Data is actually queryable.
        let conn = Connection::open(&db).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM people_list_csv WHERE age > 26",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn dedupes_and_fills_blank_headers_and_replaces() {
        let db = temp_db();
        // Duplicate + blank headers, ragged row.
        ingest_csv(&db, "t", "a,a,\n1,2,3\n").unwrap();
        let s = ingest_csv(&db, "t", "x\n10\n20\n").unwrap(); // re-upload replaces
        assert_eq!(s.row_count, 2);
        assert_eq!(s.columns.len(), 1);
        let conn = Connection::open(&db).unwrap();
        let sum: i64 = conn
            .query_row("SELECT SUM(x) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sum, 30);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn rejects_memory_db() {
        assert!(ingest_csv(":memory:", "t", "a\n1\n").is_err());
    }
}
