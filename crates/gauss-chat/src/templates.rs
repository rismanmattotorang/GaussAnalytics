//! The built-in web UI: a single, self-contained HTML/CSS/JS chat app served at
//! `/`. It speaks the GaussAnalytics SSE protocol directly (POST `chat_sse`, parse
//! `data:` frames, stop on `[DONE]`) and renders the component types the agent
//! emits — text, cards, dataframes, status bar, task tracker, charts, etc.
//!
//! UX: starter example prompts, a live "thinking" indicator, cancel/stop,
//! copy-to-clipboard for SQL/tables, smart auto-scroll, an auto-growing input,
//! ARIA live regions, and a responsive layout — with no external/CDN
//! dependencies, so it works offline and in air-gapped deploys.

pub fn index_html() -> String {
    INDEX_HTML.to_string()
}

const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>GaussAnalytics</title>
<style>
  :root {
    /* Gaussian Technologies identity: cobalt-blue mark, deep-navy surfaces, corporate gray. */
    --bg: #0e1117; --panel: #151924; --panel2: #1d2331; --border: #29313f;
    --text: #e8ebf1; --muted: #98a2b3;
    --brand: #2b5bb5;        /* logo blue */
    --brand-light: #4c7fe0;  /* lifted blue for text/links on dark */
    --brand-deep: #1c4488;   /* shaded blue */
    --accent: var(--brand-light);
    --accent2: var(--brand-deep);
    --corp: #6e7378;         /* the "CORP" gray */
    --ok: #3fb950; --warn: #d29922; --err: #f85149;
  }
  * { box-sizing: border-box; }
  html, body { height: 100%; margin: 0; }
  body {
    font: 15px/1.5 system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
    background: var(--bg); color: var(--text); display: flex; flex-direction: column;
  }
  header {
    padding: 11px 18px; border-bottom: 1px solid var(--border);
    display: flex; align-items: center; gap: 12px; background: var(--panel);
  }
  header .mark { width: 30px; height: 28px; flex: none; }
  header .brand { display: flex; flex-direction: column; line-height: 1.15; }
  header .logo { font-weight: 700; letter-spacing: .3px; font-size: 16px; }
  header .logo span { color: var(--brand-light); }
  header .sub { color: var(--muted); font-size: 11px; }
  header .sub .corp { color: var(--text); font-weight: 600; letter-spacing: .2px; }
  header .sub .corp span { color: var(--corp); }
  header .spacer { margin-left: auto; }
  header button.ghost {
    background: transparent; color: var(--muted); border: 1px solid var(--border);
    border-radius: 8px; padding: 5px 10px; font: inherit; font-size: 13px; cursor: pointer;
  }
  header button.ghost:hover { color: var(--text); border-color: var(--accent); }
  #statusbar { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--muted); }
  #statusdot { width: 9px; height: 9px; border-radius: 50%; background: var(--muted); flex: none; }
  #statusdot.working { background: var(--accent); animation: pulse 1s infinite; }
  #statusdot.idle, #statusdot.success { background: var(--ok); }
  #statusdot.error { background: var(--err); }
  #statusdot.warning { background: var(--warn); }
  @keyframes pulse { 0%,100% { opacity: 1; } 50% { opacity: .3; } }
  #main { flex: 1; display: flex; min-height: 0; position: relative; }
  #transcript { flex: 1; overflow-y: auto; padding: 18px; display: flex; flex-direction: column; gap: 12px; scroll-behavior: smooth; }
  #sidebar {
    width: 260px; border-left: 1px solid var(--border); background: var(--panel);
    padding: 14px; overflow-y: auto; font-size: 13px;
  }
  #sidebar h3 { margin: 0 0 8px; font-size: 12px; text-transform: uppercase; color: var(--muted); letter-spacing: .5px; }
  .task { display: flex; gap: 8px; padding: 5px 0; align-items: baseline; }
  .task .ic { width: 14px; flex: none; }
  .task.completed { color: var(--muted); }
  .role { font-size: 11px; text-transform: uppercase; letter-spacing: .5px; color: var(--muted); margin-bottom: 3px; }
  .msg { max-width: 82%; padding: 10px 14px; border-radius: 12px; word-wrap: break-word; overflow-wrap: anywhere; }
  .msg.user { align-self: flex-end; background: var(--brand); color: #fff; border-bottom-right-radius: 3px; }
  .msg.bot { align-self: flex-start; background: var(--panel2); border: 1px solid var(--border); border-bottom-left-radius: 3px; }
  .msg pre { background: #0b0d13; border: 1px solid var(--border); border-radius: 8px; padding: 10px; overflow-x: auto; margin: 8px 0; }
  .msg code { background: #0b0d13; padding: 1px 5px; border-radius: 4px; font-size: 13px; }
  .codewrap { position: relative; }
  .copy { position: absolute; top: 12px; right: 8px; font-size: 11px; padding: 2px 8px; border-radius: 6px;
    background: var(--panel); color: var(--muted); border: 1px solid var(--border); cursor: pointer; }
  .copy:hover { color: var(--text); border-color: var(--accent); }
  .thinking .dots { display: inline-flex; gap: 4px; }
  .thinking .dots i { width: 6px; height: 6px; border-radius: 50%; background: var(--muted); animation: blink 1.2s infinite; }
  .thinking .dots i:nth-child(2) { animation-delay: .2s; }
  .thinking .dots i:nth-child(3) { animation-delay: .4s; }
  @keyframes blink { 0%,80%,100% { opacity: .25; } 40% { opacity: 1; } }
  .card { align-self: flex-start; max-width: 92%; background: var(--panel2); border: 1px solid var(--border); border-radius: 10px; padding: 12px 14px; }
  .card .title { font-weight: 600; margin-bottom: 6px; }
  .card.status-success { border-left: 3px solid var(--ok); }
  .card.status-error { border-left: 3px solid var(--err); }
  .card.status-running, .card.status-working { border-left: 3px solid var(--accent); }
  .dfwrap { align-self: flex-start; max-width: 100%; }
  .dfscroll { max-height: 340px; overflow: auto; border: 1px solid var(--border); border-radius: 8px; }
  table.df { border-collapse: collapse; font-size: 13px; }
  table.df th, table.df td { border-bottom: 1px solid var(--border); padding: 5px 10px; text-align: left; white-space: nowrap; }
  table.df th { background: var(--panel2); position: sticky; top: 0; }
  table.df tbody tr:nth-child(even) { background: #141823; }
  .dfmeta { display: flex; gap: 10px; align-items: center; color: var(--muted); font-size: 12px; padding: 4px 2px; }
  .dfmeta button { background: transparent; border: 1px solid var(--border); border-radius: 6px; color: var(--muted); cursor: pointer; font-size: 11px; padding: 1px 7px; }
  .bar-row { display: flex; align-items: center; gap: 8px; margin: 2px 0; font-size: 12px; }
  .bar-row .lbl { width: 120px; text-align: right; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bar-row .bar { height: 14px; background: linear-gradient(90deg, var(--accent), var(--accent2)); border-radius: 3px; }
  .badge { display: inline-block; padding: 2px 8px; border-radius: 10px; background: var(--panel2); border: 1px solid var(--border); font-size: 12px; }
  .note { color: var(--muted); font-size: 13px; align-self: flex-start; }
  /* GenBI result panel: summary, auto-chart, follow-up suggestions. */
  .summary { color: var(--text); font-size: 13px; margin: 2px 0 8px; }
  .summary .reason { color: var(--muted); }
  .genbi { margin: 4px 0 10px; }
  .genbi .bignum { font-size: 34px; font-weight: 700; color: var(--brand-light); }
  .genbi svg { display: block; max-width: 100%; }
  .genbi .legend { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 6px; color: var(--muted); font-size: 12px; }
  .genbi .legend i { display: inline-block; width: 10px; height: 10px; border-radius: 2px; margin-right: 4px; vertical-align: middle; }
  .suggest { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }
  .suggest .chip { font-size: 12px; padding: 6px 11px; }
  /* Empty / starter state */
  #empty { margin: auto; max-width: 560px; text-align: center; color: var(--muted); padding: 24px; }
  #empty .mark-lg { width: 64px; height: 60px; margin-bottom: 14px; }
  #empty h2 { color: var(--text); margin: 0 0 6px; }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; justify-content: center; margin-top: 16px; }
  .chip { background: var(--panel2); border: 1px solid var(--border); color: var(--text); border-radius: 18px;
    padding: 8px 14px; font: inherit; font-size: 13px; cursor: pointer; }
  .chip:hover { border-color: var(--accent); }
  #tobottom { position: absolute; bottom: 16px; left: 50%; transform: translateX(-50%); display: none;
    background: var(--brand); color: #fff; border: 0; border-radius: 16px; padding: 6px 14px; cursor: pointer; font-size: 13px;
    box-shadow: 0 2px 8px rgba(0,0,0,.4); }
  #composer { display: flex; gap: 10px; padding: 14px 18px; border-top: 1px solid var(--border); background: var(--panel); align-items: flex-end; }
  #input { flex: 1; resize: none; max-height: 160px; background: var(--panel2); color: var(--text);
    border: 1px solid var(--border); border-radius: 10px; padding: 11px 13px; font: inherit; line-height: 1.4; }
  #input:focus { outline: none; border-color: var(--accent); }
  .uploadbtn { background: var(--panel2); color: var(--muted); border: 1px solid var(--border); border-radius: 10px; height: 44px; padding: 0 14px; font: inherit; font-weight: 600; cursor: pointer; flex: none; white-space: nowrap; }
  .uploadbtn:hover { color: var(--text); border-color: var(--brand-light); }
  #send { background: var(--brand); color: #fff; border: 0; border-radius: 10px; padding: 0 20px; height: 44px; font-weight: 600; cursor: pointer; flex: none; }
  #send:hover { background: var(--brand-light); }
  #send.stop { background: var(--err); }
  #send.stop:hover { background: var(--err); }
  #send:disabled { opacity: .5; cursor: default; }
  .hint { color: var(--muted); font-size: 11px; padding: 0 18px 8px; background: var(--panel); }
  @media (max-width: 720px) {
    #sidebar { display: none; }
    .msg, .card { max-width: 100%; }
  }
</style>
</head>
<body>
  <header>
    <svg class="mark" viewBox="0 0 100 92" aria-label="Gaussian Technologies" role="img" xmlns="http://www.w3.org/2000/svg">
      <polygon points="6,7 94,7 50,46" fill="#4c7fe0"/>
      <polygon points="6,7 50,46 50,90" fill="#2b5bb5"/>
      <polygon points="94,7 50,46 50,90" fill="#1c4488"/>
      <g stroke="#151924" stroke-width="3.2" stroke-linecap="round" fill="none">
        <path d="M50 46 L6 7 M50 46 L94 7 M50 46 L50 90"/>
      </g>
    </svg>
    <div class="brand">
      <div class="logo">Gauss<span>Analytics</span></div>
      <div class="sub">by <span class="corp">GAUSSIAN<span>&nbsp;TECH</span></span> &middot; natural language &rarr; SQL</div>
    </div>
    <div class="spacer"></div>
    <div id="statusbar" role="status" aria-live="polite"><span id="statusdot"></span><span id="statustext">Ready</span></div>
    <button id="newchat" class="ghost" aria-label="Start a new conversation">New chat</button>
  </header>
  <div id="main">
    <div id="transcript" role="log" aria-live="polite" aria-relevant="additions" aria-label="Conversation"></div>
    <button id="tobottom" aria-label="Scroll to latest">&darr; Latest</button>
    <div id="sidebar"><h3>Tasks</h3><div id="tasks"><div class="note">Idle</div></div></div>
  </div>
  <div id="composer">
    <button id="upload" class="uploadbtn" aria-label="Upload a CSV file" title="Upload a CSV file to query">&#43; CSV</button>
    <input id="file" type="file" accept=".csv,text/csv" hidden aria-hidden="true" />
    <textarea id="input" rows="1" aria-label="Ask a question" placeholder="Ask a question about your data…  (Enter to send, Shift+Enter for newline)"></textarea>
    <button id="send" aria-label="Send message">Send</button>
  </div>
  <div class="hint">Enter to send · Shift+Enter for a new line · Esc to stop streaming · <strong>+ CSV</strong> to upload &amp; query your own data</div>
<script>
(function () {
  const SSE_ENDPOINT = "/api/gauss/v2/chat_sse";
  const UPLOAD_ENDPOINT = "/api/gauss/v2/upload_csv";
  const EXAMPLES = [
    "How many customers are there?",
    "Top 3 customers by lifetime value",
    "Total order amount by status",
    "Which countries have the most customers?",
  ];
  const transcript = document.getElementById("transcript");
  const tasksEl = document.getElementById("tasks");
  const input = document.getElementById("input");
  const sendBtn = document.getElementById("send");
  const uploadBtn = document.getElementById("upload");
  const fileInput = document.getElementById("file");
  const newChatBtn = document.getElementById("newchat");
  const toBottomBtn = document.getElementById("tobottom");
  const statusDot = document.getElementById("statusdot");
  const statusText = document.getElementById("statustext");
  let conversationId = null;
  let streaming = false;
  let controller = null;
  let thinkingEl = null;
  let stick = true;
  const tasks = new Map();

  function esc(s) {
    return String(s == null ? "" : s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
  function md(s) {
    let t = esc(s);
    t = t.replace(/```(\w*)\n?([\s\S]*?)```/g, (_, _l, c) => '<div class="codewrap"><pre>' + c.replace(/\n$/, "") + "</pre></div>");
    t = t.replace(/`([^`]+)`/g, "<code>$1</code>");
    t = t.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
    t = t.replace(/\n/g, "<br>");
    return t;
  }
  function clearEmpty() { const e = document.getElementById("empty"); if (e) e.remove(); }
  function add(el) {
    clearEmpty();
    transcript.appendChild(el);
    if (stick) transcript.scrollTop = transcript.scrollHeight;
    addCopyButtons(el);
    return el;
  }
  function bubble(cls, html) {
    const d = document.createElement("div");
    d.className = "msg " + cls;
    if (cls.indexOf("bot") >= 0) html = '<div class="role">GaussAnalytics</div>' + html;
    d.innerHTML = html;
    return add(d);
  }
  // Attach copy buttons to any <pre> blocks just added.
  function addCopyButtons(el) {
    el.querySelectorAll(".codewrap").forEach(w => {
      if (w.querySelector(".copy")) return;
      const pre = w.querySelector("pre");
      if (!pre) return;
      const b = document.createElement("button");
      b.className = "copy"; b.type = "button"; b.textContent = "Copy"; b.setAttribute("aria-label", "Copy code");
      b.onclick = () => copyText(pre.innerText, b);
      w.appendChild(b);
    });
  }
  async function copyText(text, btn) {
    try { await navigator.clipboard.writeText(text); }
    catch (e) {
      const ta = document.createElement("textarea"); ta.value = text; document.body.appendChild(ta);
      ta.select(); try { document.execCommand("copy"); } catch (_) {} ta.remove();
    }
    if (btn) { const old = btn.textContent; btn.textContent = "Copied"; setTimeout(() => (btn.textContent = old), 1200); }
  }

  function setStatus(status, message) {
    statusDot.className = ""; statusDot.classList.add(status || "idle");
    statusText.textContent = message || "";
  }
  function showThinking() { if (!thinkingEl) thinkingEl = bubble("bot thinking", '<span class="dots"><i></i><i></i><i></i></span>'); }
  function hideThinking() { if (thinkingEl) { thinkingEl.remove(); thinkingEl = null; } }

  function emptyState() {
    tasks.clear(); renderTasks();
    transcript.innerHTML =
      '<div id="empty">' +
      '<svg class="mark-lg" viewBox="0 0 100 92" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">' +
      '<polygon points="6,7 94,7 50,46" fill="#4c7fe0"/><polygon points="6,7 50,46 50,90" fill="#2b5bb5"/>' +
      '<polygon points="94,7 50,46 50,90" fill="#1c4488"/>' +
      '<g stroke="#0e1117" stroke-width="3.2" stroke-linecap="round" fill="none"><path d="M50 46 L6 7 M50 46 L94 7 M50 46 L50 90"/></g></svg>' +
      '<h2>Ask your data anything</h2>' +
      '<div>Natural-language questions become governed, validated SQL — with tables and charts streamed back live.</div>' +
      '<div class="chips">' + EXAMPLES.map(e => '<button class="chip" type="button">' + esc(e) + "</button>").join("") + "</div></div>";
    transcript.querySelectorAll(".chip").forEach(c => (c.onclick = () => send(c.textContent)));
  }

  function renderTasks() {
    if (tasks.size === 0) { tasksEl.innerHTML = '<div class="note">Idle</div>'; return; }
    tasksEl.innerHTML = "";
    for (const t of tasks.values()) {
      const ic = t.status === "completed" ? "&#10003;" : t.status === "error" ? "&#10007;" : "&#8226;";
      const row = document.createElement("div");
      row.className = "task " + (t.status || "");
      row.innerHTML = '<span class="ic">' + ic + "</span><span>" + esc(t.title) + "</span>";
      tasksEl.appendChild(row);
    }
  }

  // Brand-tinted categorical palette for the no-CDN inline charts.
  const PALETTE = ["#4c7fe0","#3fb950","#d29922","#f85149","#a371f7","#2b5bb5","#1c9c8c","#e06c9f"];
  const svgNS = "http://www.w3.org/2000/svg";
  function svgEl(name, attrs) {
    const e = document.createElementNS(svgNS, name);
    for (const k in attrs) e.setAttribute(k, attrs[k]);
    return e;
  }
  function numOf(v) { const n = Number(v); return isNaN(n) ? null : n; }

  // Draw the recommended chart from its compact encoding + the result rows —
  // entirely client-side, no Vega/CDN. Returns a DOM node, or null.
  function genbiChart(chart, rows, cols) {
    if (!chart || !chart.encoding || !rows.length) return null;
    const enc = chart.encoding, type = chart.chart_type;
    const wrap = document.createElement("div"); wrap.className = "genbi";
    const W = 520, H = 220, padL = 44, padB = 28, padT = 8, padR = 8;

    if (type === "number") {
      const v = rows[0][enc.value];
      const d = document.createElement("div"); d.className = "bignum"; d.textContent = v == null ? "—" : v;
      wrap.appendChild(d); return wrap;
    }
    if (type === "bar" || type === "grouped_bar") {
      const labelFor = r => type === "grouped_bar" && enc.series ? (r[enc.x] + " · " + r[enc.series]) : r[enc.x];
      const items = rows.slice(0, 30).map(r => ({ l: String(labelFor(r) ?? ""), v: numOf(r[enc.y]) ?? 0 }));
      const max = Math.max.apply(null, items.map(i => i.v).concat([1]));
      items.forEach((it, i) => {
        const row = document.createElement("div"); row.className = "bar-row";
        const w = Math.max(2, Math.round((it.v / max) * 240));
        row.innerHTML = '<span class="lbl">' + esc(it.l) + '</span><span class="bar" style="width:' + w + 'px;background:' + PALETTE[0] + '"></span><span>' + esc(it.v) + "</span>";
        wrap.appendChild(row);
      });
      return wrap;
    }
    if (type === "pie") {
      const items = rows.slice(0, 12).map((r, i) => ({ l: String(r[enc.x] ?? ""), v: Math.max(0, numOf(r[enc.y]) ?? 0), c: PALETTE[i % PALETTE.length] }));
      const total = items.reduce((s, i) => s + i.v, 0) || 1;
      const cx = 90, cy = 90, rad = 80; let a0 = -Math.PI / 2;
      const svg = svgEl("svg", { viewBox: "0 0 360 180", width: 360, height: 180 });
      items.forEach(it => {
        const a1 = a0 + (it.v / total) * Math.PI * 2;
        const x0 = cx + rad * Math.cos(a0), y0 = cy + rad * Math.sin(a0);
        const x1 = cx + rad * Math.cos(a1), y1 = cy + rad * Math.sin(a1);
        const large = (a1 - a0) > Math.PI ? 1 : 0;
        const path = svgEl("path", { d: `M${cx},${cy} L${x0},${y0} A${rad},${rad} 0 ${large} 1 ${x1},${y1} Z`, fill: it.c });
        svg.appendChild(path); a0 = a1;
      });
      wrap.appendChild(svg);
      const leg = document.createElement("div"); leg.className = "legend";
      items.forEach(it => { const s = document.createElement("span"); s.innerHTML = '<i style="background:' + it.c + '"></i>' + esc(it.l) + " (" + Math.round(it.v / total * 100) + "%)"; leg.appendChild(s); });
      wrap.appendChild(leg); return wrap;
    }
    if (type === "line" || type === "multi_line" || type === "scatter") {
      const seriesKey = enc.series;
      const groups = {};
      rows.forEach(r => { const k = seriesKey ? String(r[seriesKey] ?? "") : "series"; (groups[k] = groups[k] || []).push(r); });
      const xsAll = rows.map(r => r[enc.x]);
      const xNum = xsAll.every(v => numOf(v) != null);
      const xVals = xsAll.map((v, i) => xNum ? numOf(v) : i);
      const yVals = rows.map(r => numOf(r[enc.y])).filter(v => v != null);
      const xMin = Math.min.apply(null, xVals), xMax = Math.max.apply(null, xVals);
      const yMin = Math.min.apply(null, yVals.concat([0])), yMax = Math.max.apply(null, yVals.concat([1]));
      const sx = v => padL + ((v - xMin) / ((xMax - xMin) || 1)) * (W - padL - padR);
      const sy = v => H - padB - ((v - yMin) / ((yMax - yMin) || 1)) * (H - padT - padB);
      const svg = svgEl("svg", { viewBox: `0 0 ${W} ${H}`, width: W, height: H });
      svg.appendChild(svgEl("line", { x1: padL, y1: H - padB, x2: W - padR, y2: H - padB, stroke: "#29313f" }));
      svg.appendChild(svgEl("line", { x1: padL, y1: padT, x2: padL, y2: H - padB, stroke: "#29313f" }));
      const keys = Object.keys(groups);
      keys.forEach((k, gi) => {
        const color = PALETTE[gi % PALETTE.length];
        const pts = groups[k].map(r => {
          const xi = xsAll.indexOf(r[enc.x]);
          return [sx(xNum ? numOf(r[enc.x]) : xi), sy(numOf(r[enc.y]) ?? 0)];
        });
        if (type === "scatter") {
          pts.forEach(p => svg.appendChild(svgEl("circle", { cx: p[0], cy: p[1], r: 3.5, fill: color })));
        } else {
          svg.appendChild(svgEl("polyline", { points: pts.map(p => p.join(",")).join(" "), fill: "none", stroke: color, "stroke-width": 2 }));
          pts.forEach(p => svg.appendChild(svgEl("circle", { cx: p[0], cy: p[1], r: 2.5, fill: color })));
        }
      });
      wrap.appendChild(svg);
      if (seriesKey && keys.length > 1) {
        const leg = document.createElement("div"); leg.className = "legend";
        keys.forEach((k, gi) => { const s = document.createElement("span"); s.innerHTML = '<i style="background:' + PALETTE[gi % PALETTE.length] + '"></i>' + esc(k); leg.appendChild(s); });
        wrap.appendChild(leg);
      }
      return wrap;
    }
    return null;
  }

  function renderDataframe(data) {
    const cols = data.columns || [];
    const rows = data.rows || [];
    const wrap = document.createElement("div"); wrap.className = "dfwrap";

    // GenBI summary (deterministic, computed server-side from the rows).
    if (data.summary) {
      const s = document.createElement("div"); s.className = "summary";
      let html = "&#129518; " + esc(data.summary);
      if (data.chart && data.chart.reason) html += ' <span class="reason">· ' + esc(data.chart.reason) + "</span>";
      s.innerHTML = html; wrap.appendChild(s);
    }
    // GenBI recommended chart (inline, no CDN).
    if (data.chart) {
      const c = genbiChart(data.chart, rows, cols);
      if (c) wrap.appendChild(c);
    }

    const meta = document.createElement("div"); meta.className = "dfmeta";
    const total = data.row_count != null ? data.row_count : rows.length;
    meta.innerHTML = "<span>" + esc(data.title || "Results") + " · " + total + " rows</span>";
    const copyBtn = document.createElement("button"); copyBtn.type = "button"; copyBtn.textContent = "Copy TSV";
    copyBtn.onclick = () => {
      const head = cols.join("\t");
      const body = rows.map(r => cols.map(c => r[c] == null ? "" : r[c]).join("\t")).join("\n");
      copyText(head + "\n" + body, copyBtn);
    };
    meta.appendChild(copyBtn);
    wrap.appendChild(meta);

    const scroll = document.createElement("div"); scroll.className = "dfscroll";
    const t = document.createElement("table"); t.className = "df";
    const thead = document.createElement("thead"); const htr = document.createElement("tr");
    cols.forEach(c => { const th = document.createElement("th"); th.textContent = c; htr.appendChild(th); });
    thead.appendChild(htr); t.appendChild(thead);
    const tb = document.createElement("tbody");
    rows.slice(0, 500).forEach(r => {
      const tr = document.createElement("tr");
      cols.forEach(c => { const td = document.createElement("td"); const v = r[c]; td.textContent = v == null ? "" : v; tr.appendChild(td); });
      tb.appendChild(tr);
    });
    t.appendChild(tb); scroll.appendChild(t); wrap.appendChild(scroll);

    // GenBI follow-up suggestions: grounded, one-click to re-ask.
    if (Array.isArray(data.suggestions) && data.suggestions.length) {
      const sug = document.createElement("div"); sug.className = "suggest";
      data.suggestions.forEach(q => {
        const b = document.createElement("button"); b.type = "button"; b.className = "chip";
        b.textContent = q; b.onclick = () => send(q);
        sug.appendChild(b);
      });
      wrap.appendChild(sug);
    }
    add(wrap);
  }

  function renderChart(data) {
    const fig = data.data || {};
    const series = (fig.data && fig.data[0]) || {};
    const wrap = document.createElement("div"); wrap.className = "card";
    wrap.innerHTML = '<div class="title">&#128202; ' + esc(data.title || data.chart_type || "chart") + "</div>";
    const xs = series.x || [], ys = series.y || [];
    if (xs.length && ys.length) {
      const max = Math.max.apply(null, ys.map(Number).filter(n => !isNaN(n))) || 1;
      for (let i = 0; i < Math.min(xs.length, 30); i++) {
        const row = document.createElement("div"); row.className = "bar-row";
        const w = Math.max(2, Math.round((Number(ys[i]) / max) * 240));
        row.innerHTML = '<span class="lbl">' + esc(xs[i]) + '</span><span class="bar" style="width:' + w + 'px"></span><span>' + esc(ys[i]) + "</span>";
        wrap.appendChild(row);
      }
    } else {
      const n = document.createElement("div"); n.className = "note"; n.textContent = (data.chart_type || "chart") + " (no plottable series)"; wrap.appendChild(n);
    }
    add(wrap);
  }

  function renderChunk(chunk) {
    const rich = chunk.rich || {};
    const type = rich.type;
    const data = rich.data || {};
    // Any content-bearing component clears the thinking indicator.
    if (["text","card","code_block","notification","dataframe","chart","status_card","badge","icon_text"].indexOf(type) >= 0) hideThinking();
    switch (type) {
      case "text": bubble("bot", md(data.content)); break;
      case "card": bubble("bot", (data.title ? "<strong>" + esc(data.title) + "</strong><br>" : "") + md(data.content)); break;
      case "code_block": bubble("bot", '<div class="codewrap"><pre>' + esc(data.content) + "</pre></div>"); break;
      case "notification": bubble("bot", esc(data.message)); break;
      case "dataframe": renderDataframe(data); break;
      case "chart": renderChart(data); break;
      case "status_card": {
        const c = document.createElement("div"); c.className = "card status-" + (data.status || "");
        c.innerHTML = '<div class="title">' + esc(data.icon || "") + " " + esc(data.title || "") + "</div>" +
          (data.description ? '<div class="note">' + esc(data.description) + "</div>" : "");
        add(c); break;
      }
      case "status_bar_update": setStatus(data.status, data.message + (data.detail ? " — " + data.detail : "")); break;
      case "chat_input_update":
        if (data.placeholder != null) input.placeholder = data.placeholder;
        break;
      case "task_tracker_update": {
        const op = data.operation;
        if (op === "clear_tasks") tasks.clear();
        else if (op === "add_task" && data.task) tasks.set(data.task.id, { title: data.task.title, status: data.task.status });
        else if (op === "update_task" && data.task_id && tasks.has(data.task_id)) { const t = tasks.get(data.task_id); if (data.status) t.status = data.status; }
        else if (op === "remove_task" && data.task_id) tasks.delete(data.task_id);
        renderTasks(); break;
      }
      case "badge": bubble("bot", '<span class="badge">' + esc(data.text) + "</span>"); break;
      case "icon_text": bubble("bot", esc(data.icon) + " " + esc(data.text)); break;
      default: if (chunk.simple && chunk.simple.text) bubble("bot", md(chunk.simple.text)); break;
    }
  }

  function setStreaming(on) {
    streaming = on;
    sendBtn.textContent = on ? "Stop" : "Send";
    sendBtn.classList.toggle("stop", on);
    sendBtn.setAttribute("aria-label", on ? "Stop streaming" : "Send message");
  }

  async function send(message) {
    if (streaming) return;
    const text = (message != null ? message : input.value).trim();
    if (!text) return;
    bubble("user", esc(text));
    input.value = ""; autosize();
    setStreaming(true); setStatus("working", "Sending…"); showThinking();
    controller = new AbortController();
    try {
      const resp = await fetch(SSE_ENDPOINT, {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ message: text, conversation_id: conversationId }),
        signal: controller.signal,
      });
      const reader = resp.body.getReader();
      const decoder = new TextDecoder();
      let buf = "";
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        let idx;
        while ((idx = buf.indexOf("\n\n")) >= 0) {
          const frame = buf.slice(0, idx); buf = buf.slice(idx + 2);
          const line = frame.split("\n").find(l => l.startsWith("data:"));
          if (!line) continue;
          const payload = line.slice(5).trim();
          if (payload === "[DONE]") continue;
          try {
            const chunk = JSON.parse(payload);
            if (chunk.conversation_id) conversationId = chunk.conversation_id;
            renderChunk(chunk);
          } catch (e) { /* ignore malformed frame */ }
        }
      }
    } catch (e) {
      if (e.name !== "AbortError") bubble("bot", '<span style="color:var(--err)">Connection error: ' + esc(e.message) + "</span>");
    } finally {
      hideThinking(); setStreaming(false); controller = null;
      if (statusText.textContent === "Sending…") setStatus("idle", "Ready");
      input.focus();
    }
  }

  function stop() { if (controller) controller.abort(); setStatus("idle", "Stopped"); }
  function autosize() { input.style.height = "auto"; input.style.height = Math.min(input.scrollHeight, 160) + "px"; }

  // Upload a CSV → SQLite, then it is queryable in chat.
  async function uploadCsv(file) {
    if (!file || streaming) return;
    const table = file.name.replace(/\.[^.]*$/, "").replace(/[^\x20-\x7E]/g, "_") || "uploaded_table";
    bubble("user", "&#128196; Uploading <strong>" + esc(file.name) + "</strong>…");
    setStatus("working", "Importing CSV…"); showThinking();
    try {
      const text = await file.text();
      const resp = await fetch(UPLOAD_ENDPOINT, {
        method: "POST",
        headers: { "Content-Type": "text/csv", "X-Table-Name": table, "X-Filename": file.name },
        body: text,
      });
      const data = await resp.json();
      hideThinking();
      if (!resp.ok) throw new Error(data.error || ("HTTP " + resp.status));
      const cols = (data.columns || []).map(c => esc(c.name) + " <span style='color:var(--muted)'>" + esc(c.type) + "</span>").join(", ");
      bubble("bot",
        "&#9989; Imported <strong>" + esc(file.name) + "</strong> into table <code>" + esc(data.table) + "</code> — " +
        data.row_count + " row(s).<br>Columns: " + cols + "<br>Ask me anything about <code>" + esc(data.table) + "</code>.");
      setStatus("idle", "Ready");
      input.value = "Show me a sample of " + data.table; autosize(); input.focus();
    } catch (e) {
      hideThinking();
      bubble("bot", '<span style="color:var(--err)">Upload failed: ' + esc(e.message) + "</span>");
      setStatus("error", "Upload failed");
    }
  }

  uploadBtn.addEventListener("click", () => fileInput.click());
  fileInput.addEventListener("change", () => {
    const f = fileInput.files && fileInput.files[0];
    fileInput.value = ""; // allow re-uploading the same filename
    uploadCsv(f);
  });

  sendBtn.addEventListener("click", () => (streaming ? stop() : send()));
  input.addEventListener("input", autosize);
  input.addEventListener("keydown", e => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); }
    if (e.key === "Escape" && streaming) { e.preventDefault(); stop(); }
  });
  newChatBtn.addEventListener("click", () => { if (streaming) stop(); conversationId = null; emptyState(); input.focus(); });
  transcript.addEventListener("scroll", () => {
    stick = transcript.scrollTop + transcript.clientHeight >= transcript.scrollHeight - 40;
    toBottomBtn.style.display = stick ? "none" : "block";
  });
  toBottomBtn.addEventListener("click", () => { stick = true; toBottomBtn.style.display = "none"; transcript.scrollTop = transcript.scrollHeight; });

  emptyState();
  input.focus();
})();
</script>
</body>
</html>"##;
