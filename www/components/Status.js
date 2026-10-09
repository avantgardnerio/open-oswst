// GET /api/status, refreshed, as a table

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { getStatus } from "../lib/api.js";
import { uptime, kb } from "../lib/format.js";
import { useInterval } from "../hooks/useInterval.js";

export function Status({ refreshMs }) {
  const [status, setStatus] = useState(null);
  const [error, setError] = useState(null);

  useInterval(async () => {
    try {
      setStatus(await getStatus());
      setError(null);
    } catch (e) {
      setError(`Can't reach the radio (${e.message})`);
    }
  }, refreshMs);

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
  // Only once there's been one since boot: an install's progress, then how it went
  if (status.management?.last_install) {
    rows.push(["Last install", status.management.last_install]);
  }
  return html`
    <section>
      <h2>Status</h2>
      <table>
        ${rows.map(([label, value]) => html`<tr><th>${label}</th><td>${value}</td></tr>`)}
      </table>
    </section>
  `;
}
