/**
 * Pure date & summary helpers for Google Calendar integration.
 */

export function dateToLocalIsoWithOffset(date, timezoneOffsetMinutesOverride = null) {
  if (!(date instanceof Date) || isNaN(date.getTime())) return "";
  const pad2 = (n) => String(n).padStart(2, "0");

  const rawOffset =
    typeof timezoneOffsetMinutesOverride === "number"
      ? timezoneOffsetMinutesOverride
      : date.getTimezoneOffset();

  let y;
  let mo;
  let d;
  let h;
  let mi;
  let s;

  if (typeof timezoneOffsetMinutesOverride === "number") {
    const shifted = new Date(date.getTime() - rawOffset * 60 * 1000);
    y = shifted.getUTCFullYear();
    mo = pad2(shifted.getUTCMonth() + 1);
    d = pad2(shifted.getUTCDate());
    h = pad2(shifted.getUTCHours());
    mi = pad2(shifted.getUTCMinutes());
    s = pad2(shifted.getUTCSeconds());
  } else {
    y = date.getFullYear();
    mo = pad2(date.getMonth() + 1);
    d = pad2(date.getDate());
    h = pad2(date.getHours());
    mi = pad2(date.getMinutes());
    s = pad2(date.getSeconds());
  }

  // getTimezoneOffset() is UTC - local in minutes (e.g. -60 for +01:00)
  const offsetMin = -rawOffset;
  const sign = offsetMin >= 0 ? "+" : "-";
  const absMin = Math.abs(offsetMin);
  const tzH = pad2(Math.floor(absMin / 60));
  const tzM = pad2(absMin % 60);

  return `${y}-${mo}-${d}T${h}:${mi}:${s}${sign}${tzH}:${tzM}`;
}

export function buildCalendarSummary(title, tmdbData) {
  const cleanTitle = String(title || "").trim();
  const huTitle = tmdbData?.huTitle ? String(tmdbData.huTitle).trim() : "";
  if (huTitle && huTitle.toLowerCase() !== cleanTitle.toLowerCase()) {
    return `${cleanTitle} (${huTitle})`;
  }
  return cleanTitle;
}

export function normalizeCalendarChannelId(sourceId, channelId) {
  const sid = String(sourceId || "").trim();
  const cid = String(channelId || "").trim();
  const prefix = `${sid}::`;
  if (sid && cid.startsWith(prefix)) {
    return cid.slice(prefix.length);
  }
  return cid;
}

export function buildCalendarEpgKey(sourceId, channelId, startDate) {
  if (!(startDate instanceof Date) || isNaN(startDate.getTime())) return "";
  const sid = String(sourceId || "").trim();
  const cid = normalizeCalendarChannelId(sid, channelId);
  if (!sid || !cid) return "";
  const startTs = Math.floor(startDate.getTime() / 1000);
  return `${sid}|${cid}|${startTs}`;
}

let calendarSvgCounter = 0;

/**
 * Generates the modern blue-gradient Google Calendar icon SVG with a dynamic day number (1-31).
 */
export function getGoogleCalendarIconSvg(
  day = new Date().getDate(),
  size = 16,
  className = "",
) {
  const dayNum = Math.min(31, Math.max(1, parseInt(day, 10) || new Date().getDate()));
  const uid = `gcal-${++calendarSvgCounter}`;
  const cls = className ? ` class="${className}"` : "";
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" width="${size}" height="${size}"${cls} aria-hidden="true">` +
    `<defs>` +
    `<linearGradient id="${uid}-bg" x1="8" y1="6" x2="40" y2="42" gradientUnits="userSpaceOnUse">` +
    `<stop offset="0%" stop-color="#80A4FF"/>` +
    `<stop offset="48%" stop-color="#5E8BF8"/>` +
    `<stop offset="100%" stop-color="#4575F0"/>` +
    `</linearGradient>` +
    `<linearGradient id="${uid}-top" x1="10" y1="5" x2="38" y2="14" gradientUnits="userSpaceOnUse">` +
    `<stop offset="0%" stop-color="#9CB9FF"/>` +
    `<stop offset="100%" stop-color="#769CF9"/>` +
    `</linearGradient>` +
    `</defs>` +
    `<path fill="url(#${uid}-bg)" d="M14 7.5c6.5-1.4 13.5-1.4 20 0 4.2.9 6.6 3.5 6.8 7.7.1 2.5-.8 6.4-.8 8.8s.9 6.3.8 8.8c-.2 4.2-2.6 6.8-6.8 7.7-6.5 1.4-13.5 1.4-20 0-4.2-.9-6.6-3.5-6.8-7.7-.1-2.5.8-6.4.8-8.8s-.9-6.3-.8-8.8c.2-4.2 2.6-6.8 6.8-7.7z"/>` +
    `<path fill="url(#${uid}-top)" opacity="0.55" d="M14 7.5c6.5-1.4 13.5-1.4 20 0 3.3.7 5.5 2.5 6.4 5.3-10.9-1.4-21.9-1.4-32.8 0 .9-2.8 3.1-4.6 6.4-5.3z"/>` +
    `<text x="24" y="31.5" text-anchor="middle" fill="#FFFFFF" font-family="Inter, Roboto, 'Segoe UI', Arial, sans-serif" font-size="21" font-weight="600" letter-spacing="-0.5">${dayNum}</text>` +
    `</svg>`
  );
}
