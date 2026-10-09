// Call `callback` now, then every `ms` while the component is on the page

import { useEffect } from "preact/hooks";

export function useInterval(callback, ms) {
  useEffect(() => {
    callback();
    const timer = setInterval(callback, ms);
    return () => clearInterval(timer);
  }, [ms]);
}
