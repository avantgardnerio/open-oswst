// Which page is showing: the part of the URL after # ("#/logs" -> "/logs").
// A hash, not a path, because the radio serves only the files that exist:
// index.html#/logs is still index.html, and the back button works

import { useState, useEffect } from "preact/hooks";

export function useRoute() {
  const [route, setRoute] = useState(currentRoute());
  useEffect(() => {
    const onChange = () => setRoute(currentRoute());
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return route;
}

function currentRoute() {
  return window.location.hash.replace(/^#/, "") || "/";
}
