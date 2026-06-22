import { useEffect, useState } from "react";
import { api, type AiSettings } from "../api/client";

/**
 * AI / LLM configuration page. The NL2SQL engine runs in-process against a
 * configured LLM provider. Admins edit the provider/model/base-URL/key here and
 * **Save** — the server validates, persists, and hot-swaps the translation
 * pipeline at runtime (no restart). The API key is write-only: it is never
 * returned (only whether one is set), and an empty key on save keeps the
 * stored secret.
 */
export function Settings({ token }: { token: string | null }) {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);

  // Editable form fields.
  const [enabled, setEnabled] = useState(false);
  const [provider, setProvider] = useState("mock");
  const [model, setModel] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");

  function apply(s: AiSettings) {
    setSettings(s);
    setEnabled(s.enabled);
    setProvider(s.provider);
    setModel(s.model);
    setBaseUrl(s.base_url);
    setApiKey("");
  }

  useEffect(() => {
    if (!token) return;
    api
      .aiSettings(token)
      .then(apply)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, [token]);

  async function save() {
    if (!token) return;
    setError(null);
    setSaved(false);
    setBusy(true);
    try {
      const next = await api.updateAiSettings(
        { enabled, provider, model, base_url: baseUrl, api_key: apiKey },
        token,
      );
      apply(next);
      setSaved(true);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!token) {
    return <p className="muted">Sign in as an administrator to manage AI settings.</p>;
  }
  if (!settings && !error) return <p className="muted">Loading settings…</p>;

  return (
    <div className="settings">
      <h3>AI / NL2SQL</h3>
      {error && <p className="app__error">{error}</p>}

      <div className="ds-form">
        <label>
          Enabled
          <select
            value={enabled ? "on" : "off"}
            onChange={(e) => setEnabled(e.target.value === "on")}
          >
            <option value="on">on</option>
            <option value="off">off</option>
          </select>
        </label>
        <label>
          Provider
          <select value={provider} onChange={(e) => setProvider(e.target.value)}>
            {(settings?.supported_providers ?? [provider]).map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))}
          </select>
        </label>
        <label>
          Model
          <input value={model} onChange={(e) => setModel(e.target.value)} placeholder="gpt-4o-mini" />
        </label>
        <label>
          Base URL
          <input
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="(provider default; required for litellm/vllm)"
          />
        </label>
        <label className="ds-form__uri">
          API key {settings?.has_api_key && <span className="muted">(set — leave blank to keep)</span>}
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={settings?.has_api_key ? "•••••••• (unchanged)" : "provider API key"}
          />
        </label>
        <div className="ds-form__actions">
          <button disabled={busy} onClick={save}>
            Save
          </button>
          {saved && <span className="ds-ok">Saved — pipeline reloaded.</span>}
        </div>
      </div>

      <h4>Supported providers</h4>
      <p className="ds-chips">
        {(settings?.supported_providers ?? []).map((p) => (
          <span key={p} className="badge" data-active={p === provider}>
            {p}
          </span>
        ))}
      </p>
      <p className="muted">
        OpenRouter, LiteLLM, vLLM, and Bedrock are OpenAI-compatible
        (LiteLLM/vLLM need a Base URL; Bedrock uses a gateway URL). Changes apply
        immediately and persist across restarts.
      </p>
    </div>
  );
}
