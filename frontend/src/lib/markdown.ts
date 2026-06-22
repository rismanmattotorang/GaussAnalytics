// Minimal, safe Markdown → HTML for text cards and notebook Markdown cells.
//
// The input is HTML-escaped FIRST, then a small set of inline/block patterns are
// applied, so the result is safe to inject with dangerouslySetInnerHTML — no
// user input can introduce tags or attributes. Shared by the dashboard text
// cards and the notebook Markdown cells so the renderer never drifts.
export function mdToHtml(src: string): string {
  const esc = src.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return esc
    .replace(/^### (.*)$/gm, "<h4>$1</h4>")
    .replace(/^## (.*)$/gm, "<h3>$1</h3>")
    .replace(/^# (.*)$/gm, "<h2>$1</h2>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\n/g, "<br>");
}
