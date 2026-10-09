// What the radio is and what it's doing

import { html } from "htm/preact";
import { Screen } from "../components/Screen.js";
import { Status } from "../components/Status.js";
import { Update } from "../components/Update.js";

/// How often the page asks the radio again
const REFRESH_MS = 5000;

export function RadioPage() {
  return html`
    <${Screen} refreshMs=${REFRESH_MS} />
    <${Status} refreshMs=${REFRESH_MS} />
    <${Update} />
  `;
}
