// The whole page: the title, the links to the pages, and the page the URL
// names (hooks/useRoute.js)

import { html } from "htm/preact";
import { useRoute } from "../hooks/useRoute.js";
import { Nav } from "./Nav.js";
import { RadioPage } from "../pages/RadioPage.js";
import { LogsPage } from "../pages/LogsPage.js";

export function App() {
  const route = useRoute();
  return html`
    <h1>Open OSWST</h1>
    <${Nav} route=${route} />
    ${route === "/logs" ? html`<${LogsPage} />` : html`<${RadioPage} />`}
  `;
}
