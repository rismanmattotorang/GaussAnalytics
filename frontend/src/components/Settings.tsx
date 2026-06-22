import { useEffect, useState } from "react";
import { api, type AiSettings } from "../api/client";

/**
 * AI / LLM configuration page. The NL2SQL engine runs in-process against a
 * configured LLM provider; this page reports the live configuration (with the
 * API key redacted) and the providers this build supports. Configuration is
 * supplied via `GAUSS_NL2SQL_*` environment variables, shown here so operators
 * know exactly what to set.
 */
export function Settings({ token }: { token: string | null }) {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!token) return;
    api
      .aiSettings(token)
      .then(setSettings)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, [token]);

  if (!token) {
    return <p className="muted">Sign in as an administrator to view AI settings.</p>;
  }
  if (error) return <p className="app__error">{error}</p>;
  if (!settings) return <p className="muted">Loading settings…</p>;

  return (
    <div className="settings">
      <h3>AI / NL2SQL</h3>
      <table className="data-table">
        <tbody>
          <tr>
            <td>Enabled</td>
            <td>{settings.enabled ? "yes" : "no"}</td>
          </tr>
          <tr>
            <td>Provider</td>
            <td>{settings.provider}</td>
          </tr>
          <tr>
            <td>Model</td>
            <td>{settings.model || <span className="muted">(provider default)</span>}</td>
          </tr>
          <tr>
            <td>Base URL</td>
            <td>{settings.base_url || <span className="muted">(provider default)</span>}</td>
          </tr>
          <tr>
            <td>API key</td>
            <td>{settings.has_api_key ? "configured" : <span className="muted">not set</span>}</td>
          </tr>
        </tbody>
      </table>

      <h4>Supported providers</h4>
      <p className="ds-chips">
        {settings.supported_providers.map((p) => (
          <span key={p} className="badge" data-active={p === settings.provider}>
            {p}
          </span>
        ))}
      </p>

      <h4>How to configure</h4>
      <p className="muted">
        Set these environment variables on the server, then restart:
      </p>
      <pre className="settings__env">
        {[
          "GAUSS_NL2SQL_ENABLED=true",
          "GAUSS_NL2SQL_PROVIDER=openrouter   # or openai | litellm | vllm | bedrock | anthropic | ollama | gemini | mock",
          "GAUSS_NL2SQL_MODEL=…               # provider model id",
          "GAUSS_NL2SQL_API_KEY=…             # provider key (not needed for mock/ollama)",
          "GAUSS_NL2SQL_BASE_URL=…            # OpenAI-compatible endpoint (required for litellm/vllm; the gateway URL for bedrock)",
        ].join("\n")}
      </pre>
    </div>
  );
}
