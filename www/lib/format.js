// Numbers as people read them

export function uptime(seconds) {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return hours > 0 ? `${hours} h ${minutes} min` : `${minutes} min ${seconds % 60} s`;
}

export function kb(bytes) {
  return `${(bytes / 1024).toFixed(1)} KB`;
}

/// A file's size: KB, or MB once it's that big
export function fileSize(bytes) {
  const mb = 1024 * 1024;
  return bytes >= mb ? `${(bytes / mb).toFixed(1)} MB` : kb(bytes);
}

/// A date in this browser's own time zone and style
export function dateTime(date) {
  return date.toLocaleString();
}
