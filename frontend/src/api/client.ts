// Typed client for the GaussAnalytics HTTP API.
//
// These types mirror the Rust server's JSON contract (see crates/gauss-server
// and crates/gauss-core::gql). Keeping the contract in one place is what lets
// the web UI and the Rust backend evolve independently.

export interface Health {
  status: string;
  name: string;
  version: string;
}

export type DataSourceKind = "postgres" | "mysql" | "sqlite" | "generic";

export interface Database {
  id: string;
  name: string;
  kind: DataSourceKind;
  is_synced: boolean;
  created_at: string;
}

// --- GQL: the structured query the UI builds and sends to /api/dataset/compile

export type Literal =
  | { kind: "int"; value: number }
  | { kind: "float"; value: number }
  | { kind: "text"; value: string }
  | { kind: "bool"; value: boolean }
  | { kind: "null" };

export type CompareOp = "eq" | "ne" | "lt" | "le" | "gt" | "ge";

export type Filter =
  | { type: "compare"; args: { field: string; op: CompareOp; value: Literal } }
  | { type: "like"; args: { field: string; pattern: string; case_insensitive?: boolean } }
  | { type: "in"; args: { field: string; values: Literal[] } }
  | { type: "between"; args: { field: string; low: Literal; high: Literal } }
  | { type: "is_null"; args: { field: string } }
  | { type: "is_not_null"; args: { field: string } }
  | { type: "and"; args: Filter[] }
  | { type: "or"; args: Filter[] }
  | { type: "not"; args: Filter };

export type AggFunc = "count" | "count_distinct" | "sum" | "avg" | "min" | "max";

export interface Aggregation {
  func: AggFunc;
  field?: string | null;
  alias?: string | null;
}

export interface OrderBy {
  field: string;
  direction?: "asc" | "desc";
}

export interface Query {
  source_table: string;
  fields?: string[];
  filters?: Filter[];
  aggregations?: Aggregation[];
  breakouts?: string[];
  order_by?: OrderBy[];
  limit?: number | null;
}

export type SqlParam =
  | { type: "int"; value: number }
  | { type: "float"; value: number }
  | { type: "text"; value: string }
  | { type: "bool"; value: boolean }
  | { type: "null" };

export interface CompiledQuery {
  sql: string;
  params: SqlParam[];
}

export interface GuardedQuery {
  sql: string;
  explanation?: string | null;
  confidence?: number | null;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/api${path}`, {
    headers: { "content-type": "application/json" },
    ...init,
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(body.error ?? `request failed: ${res.status}`);
  }
  return res.json() as Promise<T>;
}

export const api = {
  health: () => request<Health>("/health"),
  databases: () => request<Database[]>("/databases"),
  compile: (database_id: string, query: Query) =>
    request<CompiledQuery>("/dataset/compile", {
      method: "POST",
      body: JSON.stringify({ database_id, query }),
    }),
  nl2sql: (database_id: string, prompt: string) =>
    request<GuardedQuery>("/nl2sql", {
      method: "POST",
      body: JSON.stringify({ database_id, prompt }),
    }),
};
