/**
 * Pure date/timezone helpers for Trakt dashboard & calendars.
 * Works in both browser (window.TraktDates) and Node.js (CommonJS require).
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) {
    module.exports = factory();
  } else {
    root.TraktDates = factory();
  }
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  function pad2(n) {
    return String(n).padStart(2, '0');
  }

  function toLocalYmd(date) {
    const d = date instanceof Date ? date : new Date(date);
    return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
  }

  function addLocalDays(date, days) {
    const d = date instanceof Date ? date : new Date(date);
    return new Date(d.getFullYear(), d.getMonth(), d.getDate() + days, 0, 0, 0, 0);
  }

  function getLocalYesterdayYmd(nowDate) {
    const now = nowDate ? new Date(nowDate) : new Date();
    return toLocalYmd(addLocalDays(now, -1));
  }

  /**
   * Computes dashboard day boundaries in local calendar time:
   * - today: local today
   * - tomorrow: local today + 1
   * - furtherStart: local today + 2
   * - furtherEnd: Mon-Fri -> current week's Sunday; Sat-Sun -> next week's Sunday
   */
  function getWeekRange(localDate) {
    const d = localDate ? new Date(localDate) : new Date();
    const today = new Date(d.getFullYear(), d.getMonth(), d.getDate(), 0, 0, 0, 0);
    const dow = (today.getDay() + 6) % 7; // Monday = 0 .. Sunday = 6
    const tomorrow = addLocalDays(today, 1);
    const furtherStart = addLocalDays(today, 2);
    const endOffset = dow <= 4 ? 6 - dow : 13 - dow;
    const furtherEnd = addLocalDays(today, endOffset);

    return {
      today,
      tomorrow,
      furtherStart,
      furtherEnd,
      todayKey: toLocalYmd(today),
      tomorrowKey: toLocalYmd(tomorrow),
      furtherStartKey: toLocalYmd(furtherStart),
      furtherEndKey: toLocalYmd(furtherEnd),
    };
  }

  function compareEpisodesByStartTime(a, b) {
    const aStr = a && (a.firstAired || a.first_aired) ? (a.firstAired || a.first_aired) : '';
    const bStr = b && (b.firstAired || b.first_aired) ? (b.firstAired || b.first_aired) : '';
    const aMs = aStr ? new Date(aStr).getTime() : Number.POSITIVE_INFINITY;
    const bMs = bStr ? new Date(bStr).getTime() : Number.POSITIVE_INFINITY;
    const aValid = !isNaN(aMs);
    const bValid = !isNaN(bMs);
    if (aValid && bValid && aMs !== bMs) return aMs - bMs;
    if (aValid !== bValid) return aValid ? -1 : 1;
    const aTitle = String((a && (a.showTitle || a.title)) || '');
    const bTitle = String((b && (b.showTitle || b.title)) || '');
    const titleCmp = aTitle.localeCompare(bTitle);
    if (titleCmp !== 0) return titleCmp;
    const aSeason = (a && a.season) || 0;
    const bSeason = (b && b.season) || 0;
    if (aSeason !== bSeason) return aSeason - bSeason;
    return ((a && a.number) || 0) - ((b && b.number) || 0);
  }

  function getFurtherWeekRanges(localDate, maxDays) {
    const horizon = typeof maxDays === 'number' && maxDays > 0 ? maxDays : 30;
    const base = getWeekRange(localDate);
    const maxDate = addLocalDays(base.today, horizon);
    const maxKey = toLocalYmd(maxDate);
    const weeks = [];

    if (base.furtherStartKey > maxKey) {
      return weeks;
    }

    const firstEnd = base.furtherEnd <= maxDate ? base.furtherEnd : maxDate;
    weeks.push({
      start: base.furtherStart,
      end: firstEnd,
      startKey: base.furtherStartKey,
      endKey: toLocalYmd(firstEnd),
    });

    let end = firstEnd;
    while (true) {
      const nextStart = addLocalDays(end, 1);
      const nextStartKey = toLocalYmd(nextStart);
      if (nextStartKey > maxKey) break;
      const candidateEnd = addLocalDays(nextStart, 6);
      const nextEnd = candidateEnd <= maxDate ? candidateEnd : maxDate;
      weeks.push({
        start: nextStart,
        end: nextEnd,
        startKey: nextStartKey,
        endKey: toLocalYmd(nextEnd),
      });
      end = nextEnd;
    }

    return weeks;
  }

  function filterEpisodesByDateRange(episodes, startKey, endKey) {
    const result = [];
    for (const ep of episodes || []) {
      const airedStr = ep.firstAired || ep.first_aired;
      if (!airedStr) continue;
      const airedDate = new Date(airedStr);
      if (isNaN(airedDate.getTime())) continue;
      const dayKey = toLocalYmd(airedDate);
      if (dayKey >= startKey && dayKey <= endKey) {
        result.push(ep);
      }
    }
    result.sort(compareEpisodesByStartTime);
    return result;
  }

  function splitDashboardEpisodes(episodes, localDate) {
    const range = getWeekRange(localDate);
    const today = [];
    const tomorrow = [];
    const further = [];

    for (const ep of episodes || []) {
      const airedStr = ep.firstAired || ep.first_aired;
      if (!airedStr) continue;
      const airedDate = new Date(airedStr);
      if (isNaN(airedDate.getTime())) continue;
      const dayKey = toLocalYmd(airedDate);

      if (dayKey === range.todayKey) {
        today.push(ep);
      } else if (dayKey === range.tomorrowKey) {
        tomorrow.push(ep);
      } else if (dayKey >= range.furtherStartKey && dayKey <= range.furtherEndKey) {
        further.push(ep);
      }
    }

    today.sort(compareEpisodesByStartTime);
    tomorrow.sort(compareEpisodesByStartTime);
    further.sort(compareEpisodesByStartTime);

    return { today, tomorrow, further, range };
  }

  function groupByLocalDay(episodes) {
    const groups = new Map();
    for (const ep of episodes || []) {
      const airedStr = ep.firstAired || ep.first_aired;
      if (!airedStr) continue;
      const airedDate = new Date(airedStr);
      if (isNaN(airedDate.getTime())) continue;
      const key = toLocalYmd(airedDate);
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(ep);
    }
    return groups;
  }

  function formatLocalTime(isoStr) {
    if (!isoStr) return '';
    const d = new Date(isoStr);
    if (isNaN(d.getTime())) return '';
    return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  }

  /**
   * Formats the release day/time for a dashboard episode card:
   * - Previous local day (< today): yesterday label ("Tegnap"/"Yesterday"), or "MM.DD." (same year),
   *   or "YYYY.MM.DD." (earlier year).
   * - Today or future: local HH:MM.
   */
  function formatDashboardReleaseMeta(isoStr, nowDate, labels) {
    if (!isoStr) return '';
    const d = new Date(isoStr);
    if (isNaN(d.getTime())) return '';
    const now = nowDate ? new Date(nowDate) : new Date();
    const todayKey = toLocalYmd(now);
    const epDayKey = toLocalYmd(d);

    if (epDayKey < todayKey) {
      const yesterdayKey = toLocalYmd(addLocalDays(now, -1));
      if (epDayKey === yesterdayKey) {
        return labels && labels.yesterday ? labels.yesterday : 'Tegnap';
      }
      if (d.getFullYear() === now.getFullYear()) {
        return `${pad2(d.getMonth() + 1)}.${pad2(d.getDate())}.`;
      }
      return `${d.getFullYear()}.${pad2(d.getMonth() + 1)}.${pad2(d.getDate())}.`;
    }

    return formatLocalTime(isoStr);
  }

  function formatShortDateRange(startDate, endDate, lang) {
    const locale = lang === 'english' ? 'en-US' : 'hu-HU';
    const opts = { month: 'short', day: 'numeric' };
    const s = startDate.toLocaleDateString(locale, opts);
    const e = endDate.toLocaleDateString(locale, opts);
    return s === e ? s : `${s} – ${e}`;
  }

  function formatDayHeading(ymd, lang, labels) {
    const d = new Date(`${ymd}T00:00:00`);
    if (isNaN(d.getTime())) return ymd;
    const locale = lang === 'english' ? 'en-US' : 'hu-HU';
    const todayKey = toLocalYmd(new Date());
    const tomorrowKey = toLocalYmd(addLocalDays(new Date(), 1));
    const formatted = d.toLocaleDateString(locale, {
      year: 'numeric',
      month: 'long',
      day: 'numeric',
      weekday: 'long',
    });
    if (ymd === todayKey && labels && labels.today) {
      return `${formatted} (${labels.today})`;
    }
    if (ymd === tomorrowKey && labels && labels.tomorrow) {
      return `${formatted} (${labels.tomorrow})`;
    }
    return formatted;
  }

  function formatRelativeTime(isoStr, lang, nowDate) {
    if (!isoStr) return '';
    const d = new Date(isoStr);
    if (isNaN(d.getTime())) return '';
    const now = nowDate ? new Date(nowDate) : new Date();
    const diffMs = now.getTime() - d.getTime();
    const diffMin = Math.floor(diffMs / 60000);
    const diffHours = Math.floor(diffMs / 3600000);
    const isEn = lang === 'english';

    if (diffMin < 1) return isEn ? 'Just now' : 'Épp most';
    if (diffMin < 60) return isEn ? `${diffMin}m ago` : `${diffMin} perce`;
    if (diffHours < 24 && toLocalYmd(d) === toLocalYmd(now)) {
      return isEn ? `${diffHours}h ago` : `${diffHours} órája`;
    }
    const yesterdayKey = toLocalYmd(addLocalDays(now, -1));
    if (toLocalYmd(d) === yesterdayKey) {
      const time = formatLocalTime(isoStr);
      return isEn ? `Yesterday ${time}` : `Tegnap ${time}`;
    }
    const locale = isEn ? 'en-US' : 'hu-HU';
    return d.toLocaleDateString(locale, { month: 'short', day: 'numeric' }) + ' ' + formatLocalTime(isoStr);
  }

  function attachStickyDayHeader(scrollContainer, activeDayEl, options) {
    if (!scrollContainer || !activeDayEl) return;
    const opts = options || {};
    const groupSelector = opts.groupSelector || '.trakt-cal-day-group';
    const headingAttr = opts.headingAttr || 'dayHeading';
    const threshold = typeof opts.threshold === 'number' ? opts.threshold : 8;
    const topOffset = typeof opts.topOffset === 'number' ? opts.topOffset : 24;

    if (scrollContainer._stickyDayScrollHandler) {
      scrollContainer.removeEventListener('scroll', scrollContainer._stickyDayScrollHandler);
    }

    const updateActiveDay = () => {
      if (!activeDayEl.isConnected || !scrollContainer.isConnected) return;
      if (scrollContainer.scrollTop <= threshold) {
        activeDayEl.classList.add('u-hidden');
        activeDayEl.textContent = '';
        return;
      }
      const containerTop = scrollContainer.getBoundingClientRect().top;
      const groups = scrollContainer.querySelectorAll(groupSelector);
      let currentHeading = '';
      for (const g of groups) {
        const rect = g.getBoundingClientRect();
        if (rect.top <= containerTop + topOffset) {
          currentHeading = g.dataset[headingAttr] || '';
        } else {
          break;
        }
      }
      if (!currentHeading && groups.length > 0) {
        currentHeading = groups[0].dataset[headingAttr] || '';
      }
      if (currentHeading) {
        activeDayEl.textContent = currentHeading;
        activeDayEl.classList.remove('u-hidden');
      } else {
        activeDayEl.classList.add('u-hidden');
        activeDayEl.textContent = '';
      }
    };

    scrollContainer._stickyDayScrollHandler = updateActiveDay;
    scrollContainer.addEventListener('scroll', updateActiveDay, { passive: true });
    updateActiveDay();
  }

  return {
    toLocalYmd,
    addLocalDays,
    getLocalYesterdayYmd,
    getWeekRange,
    getFurtherWeekRanges,
    filterEpisodesByDateRange,
    compareEpisodesByStartTime,
    splitDashboardEpisodes,
    groupByLocalDay,
    formatLocalTime,
    formatDashboardReleaseMeta,
    formatShortDateRange,
    formatDayHeading,
    formatRelativeTime,
    attachStickyDayHeader,
  };
});
