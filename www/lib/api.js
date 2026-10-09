// The radio's HTTP API (src/http.rs) and its storage over WebDAV
// (src/webdav.rs). The page is served by the radio itself, so every path
// here goes back to the same radio.

/// GET /api/status: name, MAC, firmware, mode, memory...
export async function getStatus() {
  const response = await fetch("/api/status");
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response.json();
}

/// POST /api/management/update: ask the management server for an update,
/// and install it if there is one. Answers with a line of text
export async function startUpdate() {
  const response = await fetch("/api/management/update", { method: "POST" });
  return response.text();
}

/// The radio's screen as it is now, a 1-bit BMP. `stamp` makes each URL
/// new, so the browser fetches it again
export function screenshotUrl(stamp) {
  return `/api/screenshot?at=${stamp}`;
}

/// The files in a storage folder (e.g. "/fs/log/"), one level down:
/// [{ name, url, bytes, modified }], folders left out. `modified` is a Date,
/// or null when the radio had no clock when it wrote the file. A folder
/// that isn't there (logging never on) is an empty list
export async function listFiles(folder) {
  const response = await fetch(folder, { method: "PROPFIND", headers: { Depth: "1" } });
  if (response.status === 404) return [];
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const xml = new DOMParser().parseFromString(await response.text(), "application/xml");

  const files = [];
  for (const entry of xml.getElementsByTagNameNS("DAV:", "response")) {
    const url = property(entry, "href");
    // The folder itself, and folders in it, end with a /
    if (!url || url.endsWith("/")) continue;
    const modified = property(entry, "getlastmodified");
    files.push({
      name: decodeURIComponent(url.split("/").pop()),
      url,
      bytes: Number(property(entry, "getcontentlength") ?? 0),
      modified: modified ? new Date(modified) : null,
    });
  }
  return files;
}

/// The text of a DAV: property of one PROPFIND entry, or null
function property(entry, name) {
  return entry.getElementsByTagNameNS("DAV:", name)[0]?.textContent ?? null;
}
