// The radio's log files (/data/log, one per boot: core/src/logger.rs),
// newest first, each a link that saves it. The newest may still be growing:
// a download is the file as it is at that moment

import { html } from "htm/preact";
import { useState, useEffect } from "preact/hooks";
import { listFiles } from "../lib/api.js";
import { fileSize, dateTime } from "../lib/format.js";

const LOG_FOLDER = "/fs/log/";

export function LogsPage() {
  const [files, setFiles] = useState(null);
  const [error, setError] = useState(null);

  async function load() {
    setError(null);
    try {
      const listed = await listFiles(LOG_FOLDER);
      // NNNN.txt: the names sort the same way the numbers do
      listed.sort((a, b) => b.name.localeCompare(a.name));
      setFiles(listed);
    } catch (e) {
      setError(`Can't reach the radio (${e.message})`);
    }
  }

  useEffect(() => {
    load();
  }, []);

  return html`
    <section>
      <h2>Logs</h2>
      <button onClick=${load}>Refresh</button>
      ${error && html`<p class="error">${error}</p>`}
      ${!error && !files && html`<p>Loading...</p>`}
      ${files && files.length === 0 && html`<p>No log files.</p>`}
      ${files && files.length > 0 && html`<${LogTable} files=${files} />`}
    </section>
  `;
}

function LogTable({ files }) {
  return html`
    <table class="files">
      <tr><th>File</th><th>Size</th><th>Written</th></tr>
      ${files.map(
        (file) => html`
          <tr key=${file.name}>
            <td><a href=${file.url} download=${file.name}>${file.name}</a></td>
            <td>${fileSize(file.bytes)}</td>
            <td>${file.modified ? dateTime(file.modified) : "(no clock)"}</td>
          </tr>
        `
      )}
    </table>
  `;
}
