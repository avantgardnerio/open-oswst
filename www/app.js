// The radio's web page: what it is and what it's doing, from the radio's
// own HTTP API (src/http.rs). Served by the radio itself over WebDAV, from
// /fs/www/, so every request here goes back to the same radio.

import { html, render } from "htm/preact";
import { useState, useEffect } from "preact/hooks";

/// How often the page asks the radio again
const REFRESH_MS = 5000;

function App() {
  return html`
    <h1>Open OSWST</h1>
    <${Screen} />
    <${Status} />
  `;
}

/// The radio's screen, as a picture, refreshed
function Screen() {
  const [stamp, setStamp] = useState(Date.now());
  useEffect(() => {
    const timer = setInterval(() => setStamp(Date.now()), REFRESH_MS);
    return () => clearInterval(timer);
  }, []);
  // The stamp makes each URL new, so the browser fetches it again
  return html`
    <section>
      <h2>Screen</h2>
      <img class="screen" src=${`/api/screenshot?at=${stamp}`} alt="The radio's screen" />
    </section>
  `;
}

/// GET /api/status, refreshed, as a table
function Status() {
  const [status, setStatus] = useState(null);
  const [error, setError] = useState(null);

  useEffect(() => {
    async function load() {
      try {
        const response = await fetch("/api/status");
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        setStatus(await response.json());
        setError(null);
      } catch (e) {
        setError(`Can't reach the radio (${e.message})`);
      }
    }
    load();
    const timer = setInterval(load, REFRESH_MS);
    return () => clearInterval(timer);
  }, []);

  if (error) return html`<section><h2>Status</h2><p class="error">${error}</p></section>`;
  if (!status) return html`<section><h2>Status</h2><p>Loading...</p></section>`;

  const rows = [
    ["Name", status.name],
    ["MAC", status.mac],
    ["Firmware", `${status.firmware} (${status.firmware_state}, ${status.slot})`],
    ["Mode", status.mode],
    ["Settings profile", status.profile],
    ["Up", uptime(status.uptime_s)],
    ["Memory free", `${kb(status.heap_free)} (lowest ${kb(status.heap_min)})`],
    ["Management server", status.management ? `${status.management.host}:${status.management.port}` : "none"],
  ];
  return html`
    <section>
      <h2>Status</h2>
      <table>
        ${rows.map(([label, value]) => html`<tr><th>${label}</th><td>${value}</td></tr>`)}
      </table>
    </section>
  `;
}

function uptime(seconds) {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return hours > 0 ? `${hours} h ${minutes} min` : `${minutes} min ${seconds % 60} s`;
}

function kb(bytes) {
  return `${(bytes / 1024).toFixed(1)} KB`;
}

render(html`<${App} />`, document.getElementById("app"));
