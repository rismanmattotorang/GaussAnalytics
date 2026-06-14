# GaussAnalytics — Web Frontend

The GaussAnalytics web application: **React + TypeScript**, built with Vite.

This is the layer GaussAnalytics **reuses and keeps** from the proven analytics
UX — the parts worth preserving (a rich, accessible, battle-tested interface) —
while the entire backend is rebuilt in Rust. The frontend talks to the Rust
server purely over its HTTP/JSON API; it never depends on a backend language.

## Develop

```bash
pnpm install
pnpm dev          # http://localhost:5173, proxies /api → http://127.0.0.1:3000
```

Run the backend in another terminal so the API is available:

```bash
cargo run -p gaussctl -- serve
```

## Build

```bash
pnpm build        # outputs to frontend/dist
```

The Rust server serves `frontend/dist` as static assets in production (set via
`GAUSS_STATIC_DIR`).

## Where things live

- `src/api/client.ts` — the typed API client and the GQL/contract types that
  mirror the Rust server. Start here when wiring a new screen.
- `src/App.tsx` — application shell.
- `src/styles.css` — base styling/theme.

As Phase 2+ lands, the query builder, visualizations, dashboards, and admin
screens are built on top of this typed client.
