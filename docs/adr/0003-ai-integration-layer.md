# ADR 0003 — AI via integration layers (NL2SQL + MCP Gateway)

- **Status:** Accepted
- **Date:** 2026-06-14
- **Owner:** Gaussian Technologies

## Context

Gaussian Technologies already ships **NL2SQL** and **MCP Server** technologies.
GaussAnalytics should be AI-native but must not duplicate model or protocol
work, and must keep AI strictly governed.

## Decision

Build two thin, typed **integration layers** instead of reimplementing AI:

1. `gauss-nl2sql` — an `Nl2Sql` trait + HTTP client to Gaussian's NL2SQL
   service. GaussAnalytics owns the **guardrails**: it assembles grounded
   schema context, then validates/compiles the returned query through
   `gauss-query`, enforces the requesting user's permissions, and audits the
   prompt + candidate + outcome.

2. `gauss-mcp-gateway` — an `McpGateway` trait + HTTP client to Gaussian's MCP
   Servers. GaussAnalytics owns the **registry, policy/allow-listing, and
   audit** around tool discovery and invocation.

Endpoints, credentials, and timeouts are configured via `gauss-config`; no
model weights are bundled.

## Consequences

- **+** No reinvention; we ride Gaussian's roadmap for model quality.
- **+** Safety/governance is ours and uniform: AI-generated SQL runs through the
  same parameterized, permission-checked path as human-built queries.
- **+** MCP tool use is policy-gated and fully audited.
- **−** Hard dependency on Gaussian service availability; mitigated with
  timeouts, circuit-breaking (Phase 2+), and graceful degradation in the UI.
