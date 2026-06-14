// SQL-editor variables: `{{name}}` tokens become bound `?` parameters.

const VAR_RE = /\{\{\s*(\w+)\s*\}\}/g;

/** Distinct variable names referenced in `sql`, in first-seen order. */
export function extractVars(sql: string): string[] {
  const names: string[] = [];
  let m: RegExpExecArray | null;
  VAR_RE.lastIndex = 0;
  while ((m = VAR_RE.exec(sql)) !== null) {
    if (!names.includes(m[1])) names.push(m[1]);
  }
  return names;
}

/**
 * Replace each `{{name}}` with a positional `?` and collect the bound values in
 * order of appearance (a variable used twice yields two `?` and two params).
 */
export function substituteVars(
  sql: string,
  values: Record<string, string>,
): { sql: string; params: string[] } {
  const params: string[] = [];
  const out = sql.replace(VAR_RE, (_match, name: string) => {
    params.push(values[name] ?? "");
    return "?";
  });
  return { sql: out, params };
}
