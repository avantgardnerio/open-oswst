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

/// A date as ISO 8601 (2026-10-09 10:06:03), in this browser's time zone
export function dateTime(date) {
  const two = (number) => String(number).padStart(2, "0");
  const day = `${date.getFullYear()}-${two(date.getMonth() + 1)}-${two(date.getDate())}`;
  const time = `${two(date.getHours())}:${two(date.getMinutes())}:${two(date.getSeconds())}`;
  return `${day} ${time}`;
}
