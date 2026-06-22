// Render UNTRUSTED HTML (a kernel `text/html` output or a published notebook
// snapshot) safely. The HTML comes from arbitrary cell execution / imported
// notebooks and is shown to dashboard viewers, so it must not be able to script
// the app or read the in-memory bearer token.
//
// We render it in a sandboxed iframe with no flags: scripts are disabled and the
// document is an opaque origin with no access to the parent, cookies, or
// storage. Static rich output (e.g. pandas styled tables, HTML reprs) renders
// fine; script-driven widgets are intentionally inert. (matplotlib images flow
// through the image/png path instead, so they are unaffected.)
export function HtmlOutput({ html }: { html: string }) {
  return <iframe className="nb-html" sandbox="" srcDoc={html} title="cell HTML output" />;
}
