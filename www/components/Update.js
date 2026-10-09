// Ask the management server for an update, and install it if there is one,
// as the menu's Update does. Its progress shows in the status table (Last
// install), then the radio reboots

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { startUpdate } from "../lib/api.js";

export function Update() {
  const [answer, setAnswer] = useState(null);
  const [busy, setBusy] = useState(false);

  async function update() {
    if (!confirm("Check for an update, and install it if there is one? The radio reboots when it's done.")) return;
    setBusy(true);
    setAnswer("Asking the management server...");
    try {
      setAnswer(await startUpdate());
    } catch (e) {
      setAnswer(`Can't reach the radio (${e.message})`);
    }
    setBusy(false);
  }

  return html`
    <section>
      <h2>Update</h2>
      <button onClick=${update} disabled=${busy}>Check for update</button>
      ${answer && html`<p>${answer}</p>`}
    </section>
  `;
}
