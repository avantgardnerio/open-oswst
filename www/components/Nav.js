// Links to the pages; the one showing is marked

import { html } from "htm/preact";

const PAGES = [
  { route: "/", label: "Radio" },
  { route: "/logs", label: "Logs" },
];

export function Nav({ route }) {
  return html`
    <nav>
      ${PAGES.map(
        (page) => html`
          <a href=${`#${page.route}`} aria-current=${page.route === route ? "page" : undefined}>
            ${page.label}
          </a>
        `
      )}
    </nav>
  `;
}
