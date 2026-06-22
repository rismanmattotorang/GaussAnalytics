//! Notebook reactive-execution graph.
//!
//! Cells form a dependency graph by the variables they **define** and **use**:
//! a cell that uses a variable depends on the cell(s) that define it. A
//! topological order of that graph is a safe run order; a cycle is rejected.
//! Re-running is incremental — changing a cell only needs to re-run that cell
//! and its transitive dependents ([`downstream`]).
//!
//! This module is pure (no I/O): the server maps notebook cells onto
//! [`CellSpec`]s and asks for an order; the kernel gateway then executes.

use std::collections::{HashMap, HashSet, VecDeque};

use gauss_core::error::{CoreError, CoreResult};
use uuid::Uuid;

/// A cell reduced to its data dependencies: the variables it binds and the
/// variables it reads. Order in the input vector is the notebook's cell order,
/// used only to break ties deterministically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSpec {
    pub id: Uuid,
    pub defines: Vec<String>,
    pub uses: Vec<String>,
}

impl CellSpec {
    pub fn new(
        id: Uuid,
        defines: impl IntoIterator<Item = String>,
        uses: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            id,
            defines: defines.into_iter().collect(),
            uses: uses.into_iter().collect(),
        }
    }
}

/// Edges `from → to`: `to` depends on `from` (so `from` runs first). A cell
/// depends on every *other* cell that defines a variable it uses.
fn edges(cells: &[CellSpec]) -> Vec<(usize, usize)> {
    // variable name → indices of cells that define it.
    let mut definers: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, c) in cells.iter().enumerate() {
        for d in &c.defines {
            definers.entry(d.as_str()).or_default().push(i);
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (to, c) in cells.iter().enumerate() {
        for u in &c.uses {
            if let Some(defs) = definers.get(u.as_str()) {
                for &from in defs {
                    if from != to && seen.insert((from, to)) {
                        out.push((from, to));
                    }
                }
            }
        }
    }
    out
}

/// A topological run order over `cells` (by id), or an error if the
/// dependencies form a cycle. Ties are broken by original cell order, so the
/// result is stable and matches a reader's top-to-bottom intuition.
pub fn topo_order(cells: &[CellSpec]) -> CoreResult<Vec<Uuid>> {
    let edges = edges(cells);
    let mut indegree = vec![0usize; cells.len()];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); cells.len()];
    for &(from, to) in &edges {
        adj[from].push(to);
        indegree[to] += 1;
    }
    // Ready set kept in ascending index order for determinism.
    let mut ready: VecDeque<usize> = (0..cells.len()).filter(|&i| indegree[i] == 0).collect();
    let mut order = Vec::with_capacity(cells.len());
    while let Some(i) = pop_min(&mut ready) {
        order.push(cells[i].id);
        for &j in &adj[i] {
            indegree[j] -= 1;
            if indegree[j] == 0 {
                ready.push_back(j);
            }
        }
    }
    if order.len() != cells.len() {
        return Err(CoreError::InvalidQuery(
            "notebook cells form a dependency cycle".into(),
        ));
    }
    Ok(order)
}

/// Remove and return the smallest index in the ready set (deterministic order).
fn pop_min(ready: &mut VecDeque<usize>) -> Option<usize> {
    let min_pos = ready
        .iter()
        .enumerate()
        .min_by_key(|&(_, &v)| v)
        .map(|(pos, _)| pos)?;
    ready.remove(min_pos)
}

/// The `changed` cell plus every cell that transitively depends on it, in
/// topological order. This is the minimal set to re-run after editing `changed`.
/// Returns just `[changed]` if it has no dependents; empty if `changed` is
/// absent or the graph has a cycle.
pub fn downstream(cells: &[CellSpec], changed: Uuid) -> Vec<Uuid> {
    let Some(start) = cells.iter().position(|c| c.id == changed) else {
        return Vec::new();
    };
    let edges = edges(cells);
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); cells.len()];
    for &(from, to) in &edges {
        adj[from].push(to);
    }
    // BFS over forward edges to collect the affected set.
    let mut affected = HashSet::new();
    let mut queue = VecDeque::from([start]);
    affected.insert(start);
    while let Some(i) = queue.pop_front() {
        for &j in &adj[i] {
            if affected.insert(j) {
                queue.push_back(j);
            }
        }
    }
    // Return them in the notebook's topological order.
    match topo_order(cells) {
        Ok(order) => order
            .into_iter()
            .filter(|id| {
                cells
                    .iter()
                    .position(|c| &c.id == id)
                    .is_some_and(|idx| affected.contains(&idx))
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Heuristically extract `(defines, uses)` for a Python cell.
///
/// This is a deliberately small, dependency-free approximation (not a Python
/// parser): assignment/`for`/`def`/`class`/`import` targets are *defines*; all
/// other identifiers are *uses*. The graph builder only forms an edge when a
/// `use` matches another cell's `define`, so over-listing uses is harmless.
pub fn analyze_python(src: &str) -> (Vec<String>, Vec<String>) {
    let mut defines: Vec<String> = Vec::new();
    let mut uses: Vec<String> = Vec::new();
    let mut seen_def = HashSet::new();
    let mut seen_use = HashSet::new();

    for raw in src.lines() {
        let line = match raw.split('#').next() {
            Some(l) => l,
            None => raw,
        };
        let trimmed = line.trim_start();

        // Simple assignment: `name = ...` (not `==`, `<=`, etc.). Also handles
        // augmented assignment `name += ...` as both a def and a use.
        if let Some(name) = assignment_target(trimmed) {
            if seen_def.insert(name.clone()) {
                defines.push(name);
            }
        }
        for (kw, _) in [
            ("for ", "in"),
            ("def ", ""),
            ("class ", ""),
            ("import ", ""),
        ] {
            if let Some(rest) = trimmed.strip_prefix(kw) {
                if let Some(name) = first_ident(rest) {
                    if seen_def.insert(name.clone()) {
                        defines.push(name);
                    }
                }
            }
        }

        for ident in identifiers(line) {
            if !is_keyword(&ident) && seen_use.insert(ident.clone()) {
                uses.push(ident);
            }
        }
    }
    (defines, uses)
}

/// The identifier on the LHS of a top-level simple/augmented assignment, if any.
fn assignment_target(line: &str) -> Option<String> {
    let name = first_ident(line)?;
    let rest = line[name.len()..].trim_start();
    // `=` but not `==`; or an augmented assignment like `+=`, `-=`, `*=`.
    let eq = if let Some(after) = rest.strip_prefix('=') {
        !after.starts_with('=')
    } else {
        matches!(
            rest.chars().next(),
            Some('+' | '-' | '*' | '/' | '%' | '|' | '&')
        ) && rest[1..].starts_with('=')
    };
    eq.then_some(name)
}

/// The first identifier at the start of `s` (after trimming), if it begins one.
fn first_ident(s: &str) -> Option<String> {
    let s = s.trim_start();
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let end = s
        .char_indices()
        .find(|&(_, c)| !(c.is_ascii_alphanumeric() || c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    Some(s[..end].to_string())
}

/// All identifiers appearing in a line (attribute accesses keep only the base).
fn identifiers(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len()
                && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
            // Skip identifiers that are attribute accesses (`obj.attr` → drop attr).
            let is_attr = start > 0 && bytes[start - 1] == b'.';
            if !is_attr {
                out.push(line[start..i].to_string());
            }
        } else {
            i += 1;
        }
    }
    out
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "False"
            | "None"
            | "True"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(defines: &[&str], uses: &[&str]) -> CellSpec {
        CellSpec::new(
            Uuid::new_v4(),
            defines.iter().map(|s| s.to_string()),
            uses.iter().map(|s| s.to_string()),
        )
    }

    #[test]
    fn linear_chain_orders_by_dependency() {
        let a = spec(&["df"], &[]);
        let b = spec(&["clean"], &["df"]);
        let c = spec(&[], &["clean"]);
        let cells = vec![c.clone(), b.clone(), a.clone()]; // intentionally reversed
        let order = topo_order(&cells).unwrap();
        // a defines df → must precede b; b defines clean → must precede c.
        let pos = |id| order.iter().position(|x| *x == id).unwrap();
        assert!(pos(a.id) < pos(b.id));
        assert!(pos(b.id) < pos(c.id));
    }

    #[test]
    fn cycle_is_rejected() {
        let a = spec(&["x"], &["y"]);
        let b = spec(&["y"], &["x"]);
        assert!(topo_order(&[a, b]).is_err());
    }

    #[test]
    fn downstream_is_changed_plus_transitive_dependents() {
        let a = spec(&["df"], &[]);
        let b = spec(&["clean"], &["df"]);
        let c = spec(&[], &["clean"]);
        let d = spec(&[], &[]); // unrelated
        let cells = vec![a.clone(), b.clone(), c.clone(), d.clone()];
        let affected = downstream(&cells, a.id);
        assert_eq!(affected, vec![a.id, b.id, c.id]);
        // Editing a leaf only re-runs itself.
        assert_eq!(downstream(&cells, c.id), vec![c.id]);
        // Unrelated cell is isolated.
        assert_eq!(downstream(&cells, d.id), vec![d.id]);
    }

    #[test]
    fn python_analysis_finds_defs_and_uses() {
        let (defs, uses) =
            analyze_python("total = revenue * 1.2\nfor row in rows:\n    print(row.x)");
        assert!(defs.contains(&"total".to_string()));
        assert!(defs.contains(&"row".to_string()));
        assert!(uses.contains(&"revenue".to_string()));
        assert!(uses.contains(&"rows".to_string()));
        // `==` is not an assignment.
        let (defs2, _) = analyze_python("if a == b: pass");
        assert!(defs2.is_empty());
        // Attribute access doesn't leak `.x` as a use.
        assert!(!uses.contains(&"x".to_string()));
    }

    #[test]
    fn augmented_assignment_defines_and_uses() {
        let (defs, uses) = analyze_python("count += step");
        assert!(defs.contains(&"count".to_string()));
        assert!(uses.contains(&"step".to_string()));
    }
}
