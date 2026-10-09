// The radio's web page, from the radio's own HTTP API (src/http.rs).
// Served by the radio itself over WebDAV, from /fs/www/, so every request
// here goes back to the same radio.
//
//   components/  the parts of a page (App is the whole page)
//   pages/       one per link in the nav
//   hooks/       state shared by components: the route, refreshing
//   lib/         the radio's API, and formatting

import { html, render } from "htm/preact";
import { App } from "./components/App.js";

render(html`<${App} />`, document.getElementById("app"));
