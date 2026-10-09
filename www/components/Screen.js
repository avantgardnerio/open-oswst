// The radio's screen, as a picture, refreshed

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { screenshotUrl } from "../lib/api.js";
import { useInterval } from "../hooks/useInterval.js";

export function Screen({ refreshMs }) {
  const [stamp, setStamp] = useState(Date.now());
  useInterval(() => setStamp(Date.now()), refreshMs);
  return html`
    <section>
      <h2>Screen</h2>
      <img class="screen" src=${screenshotUrl(stamp)} alt="The radio's screen" />
    </section>
  `;
}
