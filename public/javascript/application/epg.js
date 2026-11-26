import {
  state,
  invoke,
  VIEW_HEIGHT_MAP,
  HORIZONTAL_SCALE_MAP,
} from "./state.js";
import { t, getCurrentLang } from "./i18n.js";
import { SearchUtils } from "../utils/search.js";
import {
  iconHtml,
  fuzzySearchMatch,
  openChannelEditModal,
  updateGridCornerButton,
  formatDayHeader,
  updateAllFavBtnState,
  updateLayoutButtonUi,
  renderEmptySourcesState,
  renderEmptyFavoritesState,
  escapeHtml,
  appendSafeImage,
  attachLiveSearchInput,
} from "./ui.js";
import { downloadSource } from "./sources.js";
import { toggleFavorite } from "./favorites.js";
import { hidePosterZoom, closeFixOverlay } from "../tmdb/tmdb.js";
import { openModal } from "./modal.js";
import { renderChannelOrderList } from "./channels.js";
import {
  prepareMatchesForEpg,
  isOnWatchlist,
  detachFromModal,
  isTraktMode,
  exitTraktMode,
} from "../trakt/trakt-ui.js";
import {
  prepareCalendarMatchesForEpg,
  isOnGoogleCalendar,
} from "../google/google-ui.js";
import {
  buildCalendarEpgKey,
  getGoogleCalendarIconSvg,
} from "../google/google-calendar-utils.js";

export function parseEPGDate(str, manualOffsetHours = 0) {
  if (!str || str.length < 12) return new Date();

  const y = parseInt(str.substring(0, 4), 10);
  const m = parseInt(str.substring(4, 6), 10) - 1;
  const d = parseInt(str.substring(6, 8), 10);
  const h = parseInt(str.substring(8, 10), 10);
  const min = parseInt(str.substring(10, 12), 10);
  const sec = str.length >= 14 ? parseInt(str.substring(12, 14), 10) : 0;

  let tzOffsetMin = 0;
  // Ha szerepel benne offset megadás (pl. "20260902200000 +0200")
  const spaceIdx = str.indexOf(" ");
  if (spaceIdx !== -1) {
    const tzStr = str.substring(spaceIdx + 1).trim();
    if (tzStr.length >= 5) {
      const sign = tzStr.charAt(0) === "-" ? -1 : 1;
      const tzH = parseInt(tzStr.substring(1, 3), 10);
      const tzM = parseInt(tzStr.substring(3, 5), 10);
      tzOffsetMin = sign * (tzH * 60 + tzM);
    }
  }

  // Kiszámítjuk a UTC időpontot milliszekundumban
  let utcMs = Date.UTC(y, m, d, h, min, sec) - tzOffsetMin * 60 * 1000;

  // Manuális felhasználói offset alkalmazása (órában)
  if (manualOffsetHours) {
    utcMs += manualOffsetHours * 3600 * 1000;
  }

  return new Date(utcMs);
}

export function formatDateLabel(dateStr) {
  const date = new Date(dateStr + "T00:00:00");
  const months = t("months");
  const days = t("days");

  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const target = new Date(date);
  target.setHours(0, 0, 0, 0);
  const diffDays = Math.round((target - today) / (1000 * 60 * 60 * 24));

  let dayName = days[date.getDay()];
  if (diffDays === 0) dayName = t("day_today");
  else if (diffDays === -1) dayName = t("day_yesterday");
  else if (diffDays === 1) dayName = t("day_tomorrow");

  const dayNum = date.getDate();
  if (getCurrentLang() === "english") {
    return `${dayName} - ${months[date.getMonth()]} ${dayNum}, ${date.getFullYear()}`;
  }
  return `${dayName} - ${date.getFullYear()}. ${months[date.getMonth()]} ${dayNum}.`;
}

export async function updateHeaderSortDropdown() {
  // Header sort selector removed; channel sorting is managed in Settings -> Channels.
}

export function applyActiveSortToEpgChannels() {
  if (!state.epgData || !state.epgData.channels || state.epgData.channels.length === 0) return;
  if (!state.epgData.rawChannels) {
    state.epgData.rawChannels = [...state.epgData.channels];
  }
  state.epgData.channels = [...state.epgData.rawChannels];
}

function formatLocalDateStr(d) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function buildEpgIndices(programmes) {
  const progsByChannel = new Map();
  const progsByChannelAndDate = new Map();
  const dateSet = new Set();

  for (const p of programmes) {
    let chList = progsByChannel.get(p.channel);
    if (!chList) {
      chList = [];
      progsByChannel.set(p.channel, chList);
    }
    chList.push(p);

    if (!p.start || isNaN(p.start.getTime())) continue;
    const startKey = formatLocalDateStr(p.start);
    dateSet.add(startKey);

    let chDates = progsByChannelAndDate.get(p.channel);
    if (!chDates) {
      chDates = new Map();
      progsByChannelAndDate.set(p.channel, chDates);
    }

    // Index under every local calendar day overlapped by [p.start, p.stop)
    const endValid = p.stop && !isNaN(p.stop.getTime()) && p.stop.getTime() > p.start.getTime();
    const lastDayDate = endValid ? new Date(p.stop.getTime() - 1) : p.start;
    const lastKey = formatLocalDateStr(lastDayDate);

    if (lastKey === startKey) {
      let dayList = chDates.get(startKey);
      if (!dayList) {
        dayList = [];
        chDates.set(startKey, dayList);
      }
      dayList.push(p);
    } else {
      const cursor = new Date(
        p.start.getFullYear(),
        p.start.getMonth(),
        p.start.getDate(),
      );
      for (let step = 0; step < 14; step++) {
        const key = formatLocalDateStr(cursor);
        let dayList = chDates.get(key);
        if (!dayList) {
          dayList = [];
          chDates.set(key, dayList);
        }
        dayList.push(p);
        if (key >= lastKey) break;
        cursor.setDate(cursor.getDate() + 1);
      }
    }
  }

  return {
    progsByChannel,
    progsByChannelAndDate,
    dates: Array.from(dateSet).sort(),
  };
}

let epgFetchGen = 0;
const epgSourceCache = new Map();
const MAX_CACHED_EPG_SOURCES = 3;

export function invalidateEpgSourceCache(sourceId = null) {
  if (!sourceId) {
    epgSourceCache.clear();
    return;
  }
  epgSourceCache.delete(sourceId);
  epgSourceCache.delete("__ALL_FAVORITES__");
}

function getCachedEpgSource(sourceId) {
  if (!epgSourceCache.has(sourceId)) return null;
  const entry = epgSourceCache.get(sourceId);
  epgSourceCache.delete(sourceId);
  epgSourceCache.set(sourceId, entry);
  return entry;
}

function setCachedEpgSource(sourceId, epgData) {
  if (epgSourceCache.has(sourceId)) {
    epgSourceCache.delete(sourceId);
  } else if (epgSourceCache.size >= MAX_CACHED_EPG_SOURCES) {
    const oldest = epgSourceCache.keys().next().value;
    epgSourceCache.delete(oldest);
  }
  epgSourceCache.set(sourceId, epgData);
}

export async function fetchEPG(options = {}) {
  const gen = ++epgFetchGen;
  const requestedSourceId = state.currentSourceId;

  if (!requestedSourceId) {
    renderEmptySourcesState();
    return;
  }

  updateAllFavBtnState();

  // Fast path: serve from in-memory client-side LRU cache of recent sources (0 IPC, 0 parsing)
  const cachedEntry = !options.forceReload
    ? getCachedEpgSource(requestedSourceId)
    : null;
  if (cachedEntry) {
    state.epgData = cachedEntry;
    applyActiveSortToEpgChannels();
    await Promise.all([
      prepareMatchesForEpg(state.epgData.programmes),
      prepareCalendarMatchesForEpg(),
    ]);
    if (gen !== epgFetchGen || state.currentSourceId !== requestedSourceId) {
      return;
    }
    document.getElementById("status").textContent = t("status_ready");
    setupDateSelector(options.targetDateStr || null);
    renderEPG();
    if (!options.skipScrollToNow) {
      scrollToNow();
    }
    if (
      document.getElementById("settingsOverlay").classList.contains("active")
    ) {
      renderChannelOrderList();
    }
    return;
  }

  document.getElementById("status").textContent = t("status_loading");
  try {
    let epgData = await invoke("get_epg_data", { sourceId: requestedSourceId });
    if (gen !== epgFetchGen || state.currentSourceId !== requestedSourceId) {
      return;
    }

    if (requestedSourceId === "__ALL_FAVORITES__") {
      if (!epgData.channels || epgData.channels.length === 0) {
        state.epgData = epgData;
        document.getElementById("status").textContent = t("status_ready");
        await updateHeaderSortDropdown();
        renderEmptyFavoritesState();
        return;
      }
    } else if (!epgData.channels || epgData.channels.length === 0) {
      const ok = await downloadSource(requestedSourceId, { skipFetch: true });
      if (gen !== epgFetchGen || state.currentSourceId !== requestedSourceId) {
        return;
      }
      if (ok) {
        epgData = await invoke("get_epg_data", {
          sourceId: requestedSourceId,
        });
        if (gen !== epgFetchGen || state.currentSourceId !== requestedSourceId) {
          return;
        }
      }
    }

    state.epgData = epgData;
    state.epgData.rawChannels = [...(state.epgData.channels || [])];
    await updateHeaderSortDropdown();
    applyActiveSortToEpgChannels();

    const currentSrcObj = state.sources.find((s) => s.id === requestedSourceId);
    const defaultOffset = currentSrcObj ? currentSrcObj.timeOffset || 0 : 0;

    state.epgData.programmes.forEach((p) => {
      const offset =
        requestedSourceId === "__ALL_FAVORITES__" &&
        p.timeOffset !== undefined &&
        p.timeOffset !== null
          ? p.timeOffset
          : defaultOffset;
      p.start = parseEPGDate(p.start, offset);
      p.stop = parseEPGDate(p.stop, offset);
    });

    const indices = buildEpgIndices(state.epgData.programmes);
    state.epgData.progsByChannel = indices.progsByChannel;
    state.epgData.progsByChannelAndDate = indices.progsByChannelAndDate;
    state.epgData.dates = indices.dates;

    if (state.epgData.channels && state.epgData.channels.length > 0) {
      setCachedEpgSource(requestedSourceId, state.epgData);
    }

    await Promise.all([
      prepareMatchesForEpg(state.epgData.programmes),
      prepareCalendarMatchesForEpg(),
    ]);
    if (gen !== epgFetchGen || state.currentSourceId !== requestedSourceId) {
      return;
    }

    document.getElementById("status").textContent = t("status_ready");
    setupDateSelector(options.targetDateStr || null);
    renderEPG();
    if (!options.skipScrollToNow) {
      scrollToNow();
    }

    // Cache logos in the background for channels that have a URL, no cached data, and aren't in negative cache
    const uncachedChannels = state.epgData.channels.filter((c) => {
      if (c.iconFailed) return false;
      const logoUrl = c.customIcon || c.icon;
      return (
        logoUrl &&
        !c.cachedIcon &&
        (logoUrl.startsWith("http://") || logoUrl.startsWith("https://"))
      );
    });

    if (uncachedChannels.length > 0) {
      (async () => {
        const batchSize = 3;
        for (let i = 0; i < uncachedChannels.length; i += batchSize) {
          if (gen !== epgFetchGen) break;
          const batch = uncachedChannels.slice(i, i + batchSize);
          await Promise.all(
            batch.map(async (channel) => {
              if (gen !== epgFetchGen) return;
              const logoUrl = channel.customIcon || channel.icon;
              try {
                const dataUri = await invoke("cache_logo", { url: logoUrl });
                if (gen === epgFetchGen) {
                  channel.cachedIcon = dataUri;
                  channel.iconFailed = false;
                }
              } catch (_) {
                if (gen === epgFetchGen) {
                  channel.iconFailed = true;
                }
              }
            }),
          );
        }
      })();
    }
    if (
      document.getElementById("settingsOverlay").classList.contains("active")
    ) {
      renderChannelOrderList();
    }
  } catch (err) {
    if (gen !== epgFetchGen) return;
    console.error(err);
    document.getElementById("status").textContent = t("status_error");
  }
}

function updateDateNavBtns() {
  const dateSelect = document.getElementById("dateSelect");
  const prevBtn = document.getElementById("prevDayBtn");
  const nextBtn = document.getElementById("nextDayBtn");
  if (!dateSelect) return;
  const count = dateSelect.options.length;
  const idx = dateSelect.selectedIndex;
  if (prevBtn) prevBtn.disabled = idx <= 0;
  if (nextBtn) nextBtn.disabled = idx < 0 || idx >= count - 1;
}

export function navigateDate(delta) {
  const dateSelect = document.getElementById("dateSelect");
  if (!dateSelect || dateSelect.options.length === 0) return;
  const newIdx = dateSelect.selectedIndex + delta;
  if (newIdx >= 0 && newIdx < dateSelect.options.length) {
    dateSelect.selectedIndex = newIdx;
    state.selectedDateStr = dateSelect.value;
    updateDateNavBtns();
    renderEPG();
  }
}

export function setupDateSelector(preferredDateStr = null) {
  const dateSelect = document.getElementById("dateSelect");
  if (!dateSelect) return;

  let sortedDates;
  if (Array.isArray(state.epgData.dates)) {
    sortedDates = state.epgData.dates;
  } else {
    const dates = new Set();
    state.epgData.programmes.forEach((p) => {
      if (p.start && !isNaN(p.start.getTime())) {
        dates.add(formatLocalDateStr(p.start));
      }
    });
    sortedDates = Array.from(dates).sort();
  }
  dateSelect.innerHTML = "";

  sortedDates.forEach((d) => {
    const opt = document.createElement("option");
    opt.value = d;
    opt.textContent = formatDateLabel(d);
    dateSelect.appendChild(opt);
  });

  const now = new Date();
  const todayStr = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
  if (preferredDateStr && sortedDates.includes(preferredDateStr)) {
    state.selectedDateStr = preferredDateStr;
  } else if (sortedDates.includes(todayStr)) {
    state.selectedDateStr = todayStr;
  } else if (sortedDates.length > 0) {
    state.selectedDateStr = sortedDates[0];
  }
  dateSelect.value = state.selectedDateStr;

  updateDateNavBtns();

  dateSelect.onchange = (e) => {
    state.selectedDateStr = e.target.value;
    updateDateNavBtns();
    renderEPG();
  };
}

function setupMainHeaderPointerDrag(headerItem, columnWrapper, channelId) {
  const isHorizontal = state.currentLayout === "horizontal";
  const itemSelector = isHorizontal
    ? ".channel-row-wrapper"
    : ".channel-column-wrapper";

  headerItem.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    if (
      e.target.closest(".channel-header-fav-btn") ||
      e.target.closest(".channel-header-rename-btn") ||
      e.target.closest(".channel-all-progs-badge")
    )
      return;
    let isDraggingCol = false;
    const startX = e.clientX;
    const startY = e.clientY;

    const onPointerMove = (moveEvent) => {
      const dist = Math.hypot(
        moveEvent.clientX - startX,
        moveEvent.clientY - startY,
      );
      if (!isDraggingCol && dist > 5) {
        isDraggingCol = true;
        columnWrapper.classList.add("dragging");
        try {
          headerItem.setPointerCapture(e.pointerId);
        } catch (_err) {}
      }

      if (isDraggingCol) {
        document
          .querySelectorAll(itemSelector)
          .forEach((el) => el.classList.remove("drag-over"));
        const targetCol = document
          .elementFromPoint(moveEvent.clientX, moveEvent.clientY)
          ?.closest(itemSelector);
        if (targetCol && targetCol.dataset.channelId !== channelId) {
          targetCol.classList.add("drag-over");
        }
      }
    };

    const onPointerUp = async (upEvent) => {
      document.removeEventListener("pointermove", onPointerMove);
      document.removeEventListener("pointerup", onPointerUp);
      document.removeEventListener("pointercancel", onPointerUp);

      try {
        headerItem.releasePointerCapture(e.pointerId);
      } catch (_err) {}

      columnWrapper.classList.remove("dragging");
      document
        .querySelectorAll(itemSelector)
        .forEach((el) => el.classList.remove("drag-over"));

      if (!isDraggingCol) return;
      isDraggingCol = false;

      const targetCol = document
        .elementFromPoint(upEvent.clientX, upEvent.clientY)
        ?.closest(itemSelector);
      if (
        targetCol &&
        targetCol.dataset.channelId &&
        targetCol.dataset.channelId !== channelId
      ) {
        const targetId = targetCol.dataset.channelId;
        const fromIdx = state.epgData.channels.findIndex((c) => c.id === channelId);
        const toIdx = state.epgData.channels.findIndex((c) => c.id === targetId);

        if (fromIdx !== -1 && toIdx !== -1) {
          const [moved] = state.epgData.channels.splice(fromIdx, 1);
          state.epgData.channels.splice(toIdx, 0, moved);
          state.epgData.rawChannels = [...state.epgData.channels];
          const newOrder = state.epgData.channels.map((c) => c.id);
          await invoke("save_channel_order", {
            sourceId: state.currentSourceId,
            order: newOrder,
          });
          renderEPG();
          if (
            document
              .getElementById("settingsOverlay")
              .classList.contains("active")
          ) {
            renderChannelOrderList();
          }
        }
      }
    };

    document.addEventListener("pointermove", onPointerMove);
    document.addEventListener("pointerup", onPointerUp);
    document.addEventListener("pointercancel", onPointerUp);
  });
}

let programObserver = null;

function setupProgramObserver() {
  if (programObserver) {
    programObserver.disconnect();
  }

  const viewport = document.getElementById("epgViewport");
  if (!viewport) return;

  const isHorizontal = state.currentLayout === "horizontal";
  const rootMargin = isHorizontal
    ? "300px 600px 300px 600px"
    : "600px 300px 600px 300px";

  programObserver = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        const el = entry.target;
        if (entry.isIntersecting) {
          if (!el.dataset.rendered) {
            renderProgramDetails(el);
          }
        } else {
          if (el.dataset.rendered === "true") {
            unrenderProgramDetails(el);
          }
        }
      });
    },
    {
      root: viewport,
      rootMargin,
      threshold: 0.01,
    },
  );
}

function renderProgramDetails(el) {
  const data = el._progData;
  if (!data) return;

  const { prog, isFav, isTrakt, isGoogleCal, startStr } = data;
  const cornerBadgesHtml =
    isGoogleCal || isTrakt
      ? `<div class="prog-corner-badges">${
          isGoogleCal
            ? `<span class="google-cal-epg-badge" title="${escapeHtml(t("google_calendar_epg_tooltip"))}">${getGoogleCalendarIconSvg(new Date().getDate(), 12)}</span>`
            : ""
        }${
          isTrakt
            ? `<span class="trakt-epg-badge" title="${escapeHtml(t("trakt_on_watchlist_tooltip"))}">${iconHtml("trakt-epg-badge", 12)}</span>`
            : ""
        }</div>`
      : "";

  el.innerHTML = `
          <div class="prog-header">
              <span class="start-badge">${startStr}</span>
              <span class="prog-title" title="${escapeHtml(prog.title)}">${escapeHtml(prog.title)}</span>
              <span class="fav-star ${isFav ? "active" : ""}">${iconHtml(isFav ? "star-filled" : "star", 15)}</span>
          </div>
          ${prog.desc ? `<div class="prog-desc">${escapeHtml(prog.desc)}</div>` : ""}
          ${cornerBadgesHtml}
        `;

  const star = el.querySelector(".fav-star");
  if (star) {
    star.onclick = (e) => {
      e.stopPropagation();
      toggleFavorite(prog.title);
    };
  }

  el.dataset.rendered = "true";
}

function unrenderProgramDetails(el) {
  el.innerHTML = '<div class="prog-placeholder"></div>';
  delete el.dataset.rendered;
}

function renderSingleChannelMultiDay(
  channel,
  filterText,
  isHorizontal,
  minuteScale,
) {
  const channelsContainer = document.getElementById("channelsContainer");
  channelsContainer.innerHTML = "";

  const chAllProgs = state.epgData.progsByChannel
    ? state.epgData.progsByChannel.get(channel.id) || []
    : state.epgData.programmes.filter((p) => p.channel === channel.id);
  const chDateMap = state.epgData.progsByChannelAndDate?.get(channel.id);

  const dates = new Set();
  chAllProgs.forEach((p) => {
    if (p.start && !isNaN(p.start.getTime())) {
      dates.add(formatLocalDateStr(p.start));
    }
  });

  const sortedDates = Array.from(dates).sort();
  const totalMinutes = 1440;
  const now = new Date();
  let firstMatchEl = null;

  const progQuery =
    filterText && SearchUtils
      ? SearchUtils.prepareQuery(filterText)
      : null;
  const channelQuery =
    filterText && SearchUtils
      ? SearchUtils.prepareQuery(filterText, { minFuzzyLength: 6 })
      : null;
  const chNameMatch = filterText
    ? channelQuery
      ? SearchUtils.matchPrepared(channel.name, channelQuery)
      : fuzzySearchMatch(channel.name, filterText, { minFuzzyLength: 6 })
    : false;
  const progMatchCache = filterText ? new Map() : null;
  const isProgMatch = (title) => {
    if (!filterText) return false;
    const key = title || "";
    let hit = progMatchCache.get(key);
    if (hit === undefined) {
      hit = progQuery
        ? SearchUtils.matchPrepared(key, progQuery)
        : fuzzySearchMatch(key, filterText);
      progMatchCache.set(key, hit);
    }
    return hit;
  };

  const frag = document.createDocumentFragment();
  const toObserve = [];

  sortedDates.forEach((dateStr) => {
    const dayStart = new Date(`${dateStr}T00:00:00`);
    const dayEndDate = new Date(dayStart);
    dayEndDate.setDate(dayEndDate.getDate() + 1);
    const dayEnd = dayEndDate;

    const dayProgs = chDateMap
      ? chDateMap.get(dateStr) || []
      : chAllProgs.filter((p) => p.stop > dayStart && p.start < dayEnd);

    if (filterText) {
      if (!chNameMatch && !dayProgs.some((p) => isProgMatch(p.title))) return;
    }

    const wrapper = document.createElement("div");
    wrapper.className = isHorizontal
      ? "channel-row-wrapper day-row-wrapper"
      : "channel-column-wrapper day-column-wrapper";
    wrapper.dataset.dateStr = dateStr;

    const headerItem = document.createElement("div");
    headerItem.className = isHorizontal
      ? "channel-row-header day-row-header"
      : "channel-header-item day-column-header";
    headerItem.dataset.dateStr = dateStr;

    const { fullDateStr, dayLabel, isToday } = formatDayHeader(dateStr);

    headerItem.innerHTML = `
      <div class="channel-name-badge day-header-badge ${isToday ? "today-badge" : ""}">
        <div class="day-badge-date">${fullDateStr}</div>
        <div class="day-badge-name">${dayLabel}</div>
      </div>
    `;

    // No pointer drag on day headers as date order is fixed
    wrapper.appendChild(headerItem);

    const grid = document.createElement("div");
    grid.className = isHorizontal ? "channel-row-programmes" : "programme-grid";

    dayProgs.forEach((prog) => {
      const startDiffMin =
        prog.start <= dayStart
          ? 0
          : Math.max(0, prog.start.getHours() * 60 + prog.start.getMinutes());
      const endDiffMin =
        prog.stop >= dayEnd
          ? totalMinutes
          : Math.min(
              totalMinutes,
              prog.stop.getHours() * 60 + prog.stop.getMinutes(),
            );
      const durationMin = endDiffMin - startDiffMin;

      if (durationMin <= 0) return;

      const isFav = state.favoritesSet.has(prog.title);
      const isTrakt = isOnWatchlist(prog.title);
      const progSourceId = channel?.sourceId || state.currentSourceId;
      const progChannelId = channel?.id || prog.channel;
      const isGoogleCal = isOnGoogleCalendar(
        buildCalendarEpgKey(progSourceId, progChannelId, prog.start),
      );
      const isHighlight = filterText ? isProgMatch(prog.title) : false;
      const isActual = prog.start <= now && prog.stop > now;

      const progEl = document.createElement("div");
      progEl.className = `programme ${isFav ? "favorite" : ""} ${isTrakt ? "trakt-watchlist" : ""} ${isHighlight ? "highlight" : ""} ${isActual ? "actual" : ""}`;
      if (isHorizontal) {
        progEl.style.left = `${startDiffMin * minuteScale}px`;
        progEl.style.width = `${Math.max(20, durationMin * minuteScale - 1)}px`;
      } else {
        progEl.style.top = `${startDiffMin * state.currentMinuteHeight}px`;
        progEl.style.height = `${Math.max(16, durationMin * state.currentMinuteHeight - 1)}px`;
      }

      const startStr = prog.start.toTimeString().substring(0, 5);
      const stopStr = prog.stop.toTimeString().substring(0, 5);

      progEl._progData = {
        prog,
        isFav,
        isTrakt,
        isGoogleCal,
        isHighlight,
        isActual,
        startStr,
        stopStr,
        channel,
      };
      progEl.innerHTML = '<div class="prog-placeholder"></div>';

      if (isHighlight && !firstMatchEl) {
        firstMatchEl = progEl;
      }

      progEl.onclick = () => {
        const srcObj = state.sources.find((s) => s.id === progSourceId);
        openModal(prog.title, `${startStr} - ${stopStr}`, prog.desc, channel, {
          airing: {
            sourceId: progSourceId,
            channelId: progChannelId,
            startDate: prog.start,
            stopDate: prog.stop,
            channelName: channel?.name || "",
            sourceName: channel?.sourceName || srcObj?.name || "",
          },
        });
      };

      grid.appendChild(progEl);
      toObserve.push(progEl);
    });

    wrapper.appendChild(grid);
    frag.appendChild(wrapper);
  });

  channelsContainer.appendChild(frag);
  if (programObserver) {
    toObserve.forEach((el) => programObserver.observe(el));
  }

  renderNowLine();

  if (firstMatchEl) {
    const viewport = document.getElementById("epgViewport");
    if (isHorizontal) {
      viewport.scrollLeft = Math.max(0, firstMatchEl.offsetLeft - 350);
    } else {
      viewport.scrollTop = Math.max(0, firstMatchEl.offsetTop - 150);
    }
  }
}

export function renderEPG() {
  if (!state.currentSourceId || state.sources.length === 0) {
    renderEmptySourcesState();
    return;
  }

  setupProgramObserver();

  const filterText = document
    .getElementById("searchInput")
    .value.toLowerCase()
    .trim();
  const epgGridEl = document.getElementById("epgGrid");
  const timeColumn = document.getElementById("timeColumn");
  const channelsContainer = document.getElementById("channelsContainer");
  timeColumn.innerHTML = "";
  channelsContainer.innerHTML = "";

  const isHorizontal = state.currentLayout === "horizontal";
  const minuteScale = getEffectiveMinuteScale();

  if (isHorizontal) {
    epgGridEl.classList.add("layout-horizontal");
    timeColumn.className = "timeline-row";
  } else {
    epgGridEl.classList.remove("layout-horizontal");
    timeColumn.className = "time-column";
  }

  document.documentElement.style.setProperty(
    "--minute-height",
    `${state.currentMinuteHeight}px`,
  );
  document.documentElement.style.setProperty(
    "--minute-scale",
    `${minuteScale}px`,
  );
  document.documentElement.style.setProperty("--step-min", state.currentStepMin);

  const totalMinutes = 1440;
  const timeFrag = document.createDocumentFragment();
  for (let min = 0; min < totalMinutes; min += state.currentStepMin) {
    const h = Math.floor(min / 60) % 24;
    const m = min % 60;
    const timeDiv = document.createElement("div");
    timeDiv.className = isHorizontal ? "timeline-slot" : "time-slot";
    if (m === 0) timeDiv.classList.add("full-hour");
    timeDiv.textContent = `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
    timeFrag.appendChild(timeDiv);
  }
  timeColumn.appendChild(timeFrag);

  const dateNavGroup = document.querySelector(".date-nav-group");
  if (dateNavGroup) {
    dateNavGroup.classList.toggle("u-hidden", !!state.activeSingleChannelId);
  }
  updateGridCornerButton();

  const activeBadge = document.getElementById("activeChannelBadge");
  const activeLogo = document.getElementById("activeChannelLogo");
  const activeName = document.getElementById("activeChannelName");

  if (activeBadge) {
    if (state.activeSingleChannelId) {
      const channel = state.epgData.channels.find(
        (c) => c.id === state.activeSingleChannelId,
      );
      if (channel) {
        const effectiveLogo =
          channel.cachedIcon ||
          (channel.iconFailed ? "" : channel.customIcon || channel.icon || "");
        if (activeLogo) {
          if (effectiveLogo) {
            activeLogo.src = effectiveLogo;
            activeLogo.classList.remove("u-hidden");
          } else {
            activeLogo.classList.add("u-hidden");
          }
        }
        if (activeName) {
          activeName.textContent = channel.name;
        }
        activeBadge.classList.remove("u-hidden");
        activeBadge.title = t("grid_back_btn_title");
        activeBadge.onclick = () => {
          state.activeSingleChannelId = null;
          renderEPG();
          scrollToNow();
        };
      } else {
        activeBadge.classList.add("u-hidden");
      }
    } else {
      activeBadge.classList.add("u-hidden");
    }
  }

  if (state.activeSingleChannelId) {
    const channel = state.epgData.channels.find(
      (c) => c.id === state.activeSingleChannelId,
    );
    if (channel) {
      renderSingleChannelMultiDay(channel, filterText, isHorizontal, minuteScale);
      return;
    }
    state.activeSingleChannelId = null;
    const activeBadge = document.getElementById("activeChannelBadge");
    if (activeBadge) activeBadge.classList.add("u-hidden");
  }

  const dayStart = new Date(`${state.selectedDateStr}T00:00:00`);
  // Use next day's midnight in wall-clock time (DST-safe)
  const dayEndDate = new Date(dayStart);
  dayEndDate.setDate(dayEndDate.getDate() + 1);
  const dayEnd = dayEndDate;
  const now = new Date();

  let firstMatchEl = null;

  const progQuery =
    filterText && SearchUtils
      ? SearchUtils.prepareQuery(filterText)
      : null;
  const channelQuery =
    filterText && SearchUtils
      ? SearchUtils.prepareQuery(filterText, { minFuzzyLength: 6 })
      : null;
  const progMatchCache = filterText ? new Map() : null;
  const isProgMatch = (title) => {
    if (!filterText) return false;
    const key = title || "";
    let hit = progMatchCache.get(key);
    if (hit === undefined) {
      hit = progQuery
        ? SearchUtils.matchPrepared(key, progQuery)
        : fuzzySearchMatch(key, filterText);
      progMatchCache.set(key, hit);
    }
    return hit;
  };

  const channelsFrag = document.createDocumentFragment();
  const toObserve = [];
  const progsByChDate = state.epgData.progsByChannelAndDate;

  state.epgData.channels.forEach((channel) => {
    if (channel.isHidden) return;

    const channelProgs = progsByChDate
      ? progsByChDate.get(channel.id)?.get(state.selectedDateStr) || []
      : state.epgData.programmes.filter(
          (p) => p.channel === channel.id && p.stop > dayStart && p.start < dayEnd,
        );

    if (channelProgs.length === 0) return;

    if (filterText) {
      const chNameMatch = channelQuery
        ? SearchUtils.matchPrepared(channel.name, channelQuery)
        : fuzzySearchMatch(channel.name, filterText, { minFuzzyLength: 6 });
      if (!chNameMatch && !channelProgs.some((p) => isProgMatch(p.title))) {
        return;
      }
    }

    const wrapper = document.createElement("div");
    wrapper.className = isHorizontal
      ? "channel-row-wrapper"
      : "channel-column-wrapper";
    wrapper.dataset.channelId = channel.id;

    const headerItem = document.createElement("div");
    headerItem.className = isHorizontal
      ? "channel-row-header"
      : "channel-header-item";
    headerItem.dataset.channelId = channel.id;

    // Use cached icon if available; skip broken external URLs known to be in negative cache
    const effectiveLogo =
      channel.cachedIcon ||
      (channel.iconFailed ? "" : channel.customIcon || channel.icon || "");

    const showSourceBadge =
      state.currentSourceId === "__ALL_FAVORITES__" || !!channel.sourceName;
    const sourceBadgeText = channel.sourceName || channel.countryCode || "";
    const sourceBadgeHtml =
      showSourceBadge && sourceBadgeText
        ? `<span class="channel-source-badge" title="${escapeHtml(channel.sourceName || "")}">${escapeHtml(sourceBadgeText)}</span>`
        : "";

    const nameTextGroup = `<span>${escapeHtml(channel.name)}</span>`;

    headerItem.innerHTML = `
      <div class="channel-name-badge ${channel.isFavorite ? "favorite-channel" : ""}" title="${escapeHtml(channel.originalName || channel.name)}">
          ${nameTextGroup}
      </div>
      <div class="channel-header-btns-row">
          <button class="channel-header-rename-btn" title="${escapeHtml(t("channel_edit_title"))}">${iconHtml("dots", 16)}</button>
          <button class="channel-header-fav-btn ${channel.isFavorite ? "active" : ""}" title="${escapeHtml(channel.isFavorite ? t("channel_fav_toggle_on") : t("channel_fav_toggle_off"))}">${iconHtml(channel.isFavorite ? "star-filled" : "star", 18)}</button>
      </div>
      <button class="channel-all-progs-badge" title="${escapeHtml(t("all_programs_badge_title"))}">
          <span>${iconHtml("calendar", 12)} ${escapeHtml(t("all_programs_badge"))}</span>
      </button>
      <div class="channel-header-source-col">
          ${sourceBadgeHtml}
      </div>
    `;

    if (effectiveLogo) {
      const nameBadge = headerItem.querySelector(".channel-name-badge");
      appendSafeImage(nameBadge, effectiveLogo, {
        className: "channel-header-logo",
        prepend: true,
      });
    }

    const allProgsBtn = headerItem.querySelector(".channel-all-progs-badge");
    if (allProgsBtn) {
      allProgsBtn.onclick = (e) => {
        e.stopPropagation();
        state.activeSingleChannelId = channel.id;
        renderEPG();
        scrollToNow();
      };
    }

    const renameBtn = headerItem.querySelector(".channel-header-rename-btn");
    if (renameBtn) {
      renameBtn.onclick = async (e) => {
        e.stopPropagation();
        const result = await openChannelEditModal(channel);
        if (!result) return;

        const targetSrc = channel.sourceId || state.currentSourceId;
        const targetChId = channel.id.includes("::")
          ? channel.id.split("::").slice(1).join("::")
          : channel.id;

        if (result.action === "reset") {
          await invoke("rename_channel", {
            sourceId: targetSrc,
            channelId: targetChId,
            name: "",
          });
          channel.customName = null;
          channel.name = channel.originalName || channel.name;
        } else if (result.action === "save") {
          await invoke("rename_channel", {
            sourceId: targetSrc,
            channelId: targetChId,
            name: result.name,
          });
          await invoke("set_channel_logo", {
            sourceId: targetSrc,
            channelId: targetChId,
            logoUrl: result.logo,
          });
          channel.customName = result.name || null;
          channel.name = result.name || channel.originalName || channel.name;
          channel.customIcon = result.logo || null;
          channel.cachedIcon = null;
          channel.iconFailed = false;
          if (result.logo) {
            invoke("cache_logo", { url: result.logo })
              .then((dataUri) => {
                channel.cachedIcon = dataUri;
                channel.iconFailed = false;
                renderEPG();
              })
              .catch(() => {
                channel.iconFailed = true;
              });
          }
        }

        epgSourceCache.delete(targetSrc);
        epgSourceCache.delete("__ALL_FAVORITES__");
        renderEPG();
        if (
          document
            .getElementById("settingsOverlay")
            .classList.contains("active")
        ) {
          renderChannelOrderList();
        }
      };
    }

    const favBtn = headerItem.querySelector(".channel-header-fav-btn");
    if (favBtn) {
      favBtn.onclick = async (e) => {
        e.stopPropagation();
        const targetSrc = channel.sourceId || state.currentSourceId;
        const targetChId = channel.id.includes("::")
          ? channel.id.split("::").slice(1).join("::")
          : channel.id;
        const newFav = await invoke("toggle_channel_favorite", {
          sourceId: targetSrc,
          channelId: targetChId,
        });
        channel.isFavorite = newFav;
        epgSourceCache.delete(targetSrc);
        epgSourceCache.delete("__ALL_FAVORITES__");
        if (state.currentSourceId === "__ALL_FAVORITES__" && !newFav) {
          state.epgData.channels = state.epgData.channels.filter(
            (c) => c.id !== channel.id,
          );
          if (state.epgData.channels.length === 0) {
            renderEmptyFavoritesState();
            return;
          }
        } else {
          state.epgData.channels.sort((a, b) => {
            if (a.isFavorite !== b.isFavorite) {
              return a.isFavorite ? -1 : 1;
            }
            return 0;
          });
        }
        renderEPG();
        if (
          document
            .getElementById("settingsOverlay")
            .classList.contains("active")
        ) {
          renderChannelOrderList();
        }
      };
    }

    setupMainHeaderPointerDrag(headerItem, wrapper, channel.id);
    wrapper.appendChild(headerItem);

    const grid = document.createElement("div");
    grid.className = isHorizontal ? "channel-row-programmes" : "programme-grid";

    channelProgs.forEach((prog) => {
      // Use wall-clock minutes instead of elapsed time to avoid
      // DST offset issues (e.g. fall-back day adding an extra hour)
      const startDiffMin =
        prog.start <= dayStart
          ? 0
          : Math.max(0, prog.start.getHours() * 60 + prog.start.getMinutes());
      const endDiffMin =
        prog.stop >= dayEnd
          ? totalMinutes
          : Math.min(
              totalMinutes,
              prog.stop.getHours() * 60 + prog.stop.getMinutes(),
            );
      const durationMin = endDiffMin - startDiffMin;

      if (durationMin <= 0) return;

      const isFav = state.favoritesSet.has(prog.title);
      const isTrakt = isOnWatchlist(prog.title);
      const progSourceId = channel?.sourceId || state.currentSourceId;
      const progChannelId = channel?.id || prog.channel;
      const isGoogleCal = isOnGoogleCalendar(
        buildCalendarEpgKey(progSourceId, progChannelId, prog.start),
      );
      const isHighlight = filterText ? isProgMatch(prog.title) : false;
      const isActual = prog.start <= now && prog.stop > now;

      const progEl = document.createElement("div");
      progEl.className = `programme ${isFav ? "favorite" : ""} ${isTrakt ? "trakt-watchlist" : ""} ${isHighlight ? "highlight" : ""} ${isActual ? "actual" : ""}`;
      if (isHorizontal) {
        progEl.style.left = `${startDiffMin * minuteScale}px`;
        progEl.style.width = `${Math.max(20, durationMin * minuteScale - 1)}px`;
      } else {
        progEl.style.top = `${startDiffMin * state.currentMinuteHeight}px`;
        progEl.style.height = `${Math.max(16, durationMin * state.currentMinuteHeight - 1)}px`;
      }

      const startStr = prog.start.toTimeString().substring(0, 5);
      const stopStr = prog.stop.toTimeString().substring(0, 5);

      progEl._progData = {
        prog,
        isFav,
        isTrakt,
        isGoogleCal,
        isHighlight,
        isActual,
        startStr,
        stopStr,
        channel,
      };
      progEl.innerHTML = '<div class="prog-placeholder"></div>';

      if (isHighlight && !firstMatchEl) {
        firstMatchEl = progEl;
      }

      progEl.onclick = () => {
        const srcObj = state.sources.find((s) => s.id === progSourceId);
        openModal(prog.title, `${startStr} - ${stopStr}`, prog.desc, channel, {
          airing: {
            sourceId: progSourceId,
            channelId: progChannelId,
            startDate: prog.start,
            stopDate: prog.stop,
            channelName: channel?.name || "",
            sourceName: channel?.sourceName || srcObj?.name || "",
          },
        });
      };

      grid.appendChild(progEl);
      toObserve.push(progEl);
    });

    wrapper.appendChild(grid);
    channelsFrag.appendChild(wrapper);
  });

  channelsContainer.appendChild(channelsFrag);
  if (programObserver) {
    toObserve.forEach((el) => programObserver.observe(el));
  }

  renderNowLine();

  if (firstMatchEl) {
    const viewport = document.getElementById("epgViewport");
    if (isHorizontal) {
      viewport.scrollLeft = Math.max(0, firstMatchEl.offsetLeft - 350);
    } else {
      viewport.scrollTop = Math.max(0, firstMatchEl.offsetTop - 150);
    }
  }
}

export async function jumpToEpgAiring(airing) {
  if (!airing || !airing.sourceId || !airing.channelId) return;

  const jumpOverlay = document.getElementById("epgJumpOverlay");
  const jumpOverlayText = document.getElementById("epgJumpOverlayText");
  if (jumpOverlayText) {
    const details = [airing.channelName, airing.sourceName].filter(Boolean).join(" · ");
    jumpOverlayText.textContent = details
      ? `${t("trakt_jumping_to_epg")} (${details})`
      : t("trakt_jumping_to_epg");
  }
  if (jumpOverlay) {
    jumpOverlay.classList.remove("u-hidden");
  }

  // Yield a frame so the progress layer paints immediately before heavy DOM/IPC work
  await new Promise((resolve) => requestAnimationFrame(resolve));

  try {
    const startDate = parseEPGDate(airing.start, airing.timeOffset || 0);
    const targetDateStr = `${startDate.getFullYear()}-${String(startDate.getMonth() + 1).padStart(2, "0")}-${String(startDate.getDate()).padStart(2, "0")}`;
    const startMin = startDate.getHours() * 60 + startDate.getMinutes();

    hidePosterZoom();
    closeFixOverlay();
    detachFromModal();
    document.getElementById("modalOverlay").classList.remove("active");
    document.getElementById("favoritesModalOverlay")?.classList.remove("active");

    if (isTraktMode()) {
      exitTraktMode({ skipRender: true });
    }

    state.activeSingleChannelId = null;

    const searchInput = document.getElementById("searchInput");
    if (searchInput && searchInput.value.trim()) {
      const q = searchInput.value.trim();
      if (
        !fuzzySearchMatch(airing.title || "", q) &&
        !fuzzySearchMatch(airing.channelName || "", q, { minFuzzyLength: 6 })
      ) {
        searchInput.value = "";
        document.getElementById("clearSearchBtn")?.classList.add("u-hidden");
      }
    }

    if (state.currentSourceId !== airing.sourceId) {
      state.currentSourceId = airing.sourceId;
      const sourceSelect = document.getElementById("sourceSelect");
      if (sourceSelect) sourceSelect.value = state.currentSourceId;
      updateAllFavBtnState();
      invoke("set_setting", {
        key: "current_source_id",
        value: state.currentSourceId,
      }).catch(() => {});
      await fetchEPG({ skipScrollToNow: true, targetDateStr });
    } else {
      const dateSelect = document.getElementById("dateSelect");
      if (dateSelect) {
        const hasDate = Array.from(dateSelect.options).some(
          (o) => o.value === targetDateStr,
        );
        if (hasDate) {
          state.selectedDateStr = targetDateStr;
          dateSelect.value = targetDateStr;
          updateDateNavBtns();
        }
      } else {
        state.selectedDateStr = targetDateStr;
      }
      renderEPG();
    }

    const scrollToTarget = () => {
      const viewport = document.getElementById("epgViewport");
      if (!viewport) return;

      const wrappers = Array.from(
        document.querySelectorAll(
          state.currentLayout === "horizontal"
            ? ".channel-row-wrapper"
            : ".channel-column-wrapper",
        ),
      );
      const wrapper = wrappers.find(
        (w) =>
          w.dataset.channelId === airing.channelId ||
          (w.dataset.channelId &&
            w.dataset.channelId.endsWith(`::${airing.channelId}`)),
      );

      let targetProgEl = null;
      if (wrapper) {
        const progs = Array.from(wrapper.querySelectorAll(".programme"));
        let bestDiff = Infinity;
        for (const el of progs) {
          const pd = el._progData;
          if (!pd || !pd.prog) continue;
          if (pd.prog.title === airing.title) {
            const diff = Math.abs(pd.prog.start.getTime() - startDate.getTime());
            if (diff < bestDiff) {
              bestDiff = diff;
              targetProgEl = el;
            }
          }
        }
        if (targetProgEl && !targetProgEl.dataset.rendered) {
          renderProgramDetails(targetProgEl);
        }
      }

      if (state.currentLayout === "horizontal") {
        const minuteScale = getEffectiveMinuteScale();
        const targetLeft = targetProgEl
          ? targetProgEl.offsetLeft
          : startMin * minuteScale;
        viewport.scrollLeft = Math.max(0, targetLeft - 220);
        if (wrapper) {
          viewport.scrollTop = Math.max(0, wrapper.offsetTop - 80);
        }
      } else {
        const targetTop = targetProgEl
          ? targetProgEl.offsetTop
          : startMin * state.currentMinuteHeight;
        viewport.scrollTop = Math.max(0, targetTop - 120);
        if (wrapper) {
          viewport.scrollLeft = Math.max(0, wrapper.offsetLeft - 80);
        }
      }

      if (targetProgEl) {
        targetProgEl.classList.remove("trakt-jump-flash");
        void targetProgEl.offsetWidth;
        targetProgEl.classList.add("trakt-jump-flash");
        setTimeout(() => {
          if (targetProgEl.isConnected) {
            targetProgEl.classList.remove("trakt-jump-flash");
          }
        }, 2600);
      }
    };

    scrollToTarget();
    await new Promise((resolve) =>
      requestAnimationFrame(() => {
        scrollToTarget();
        resolve();
      }),
    );
  } finally {
    if (jumpOverlay) {
      jumpOverlay.classList.add("u-hidden");
    }
  }
}

export function isFavoriteTitle(title) {
  return state.favoritesSet.has(title);
}

export function renderNowLine() {
  const oldLines = document.querySelectorAll(".now-line, .now-line-vertical");
  oldLines.forEach((l) => l.remove());

  const now = new Date();
  const todayStr = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;

  if (state.activeSingleChannelId) {
    const todayWrapper = document.querySelector(
      `[data-date-str="${todayStr}"]`,
    );
    if (!todayWrapper) return;

    const currentMin = now.getHours() * 60 + now.getMinutes();

    if (state.currentLayout === "horizontal") {
      const progGrid = todayWrapper.querySelector(".channel-row-programmes");
      if (!progGrid) return;
      const minuteScale = getEffectiveMinuteScale();
      const leftPx = currentMin * minuteScale;
      const line = document.createElement("div");
      line.className = "now-line-vertical";
      line.style.left = `${leftPx}px`;
      progGrid.appendChild(line);
    } else {
      const progGrid = todayWrapper.querySelector(".programme-grid");
      if (!progGrid) return;
      const topPx = currentMin * state.currentMinuteHeight;
      const line = document.createElement("div");
      line.className = "now-line";
      line.style.top = `${topPx}px`;
      progGrid.appendChild(line);
    }
    return;
  }

  if (state.selectedDateStr === todayStr) {
    const currentMin = now.getHours() * 60 + now.getMinutes();

    if (state.currentLayout === "horizontal") {
      const channelColWidth =
        parseInt(
          getComputedStyle(document.documentElement).getPropertyValue(
            "--channel-col-width",
          ),
        ) || 300;
      const minuteScale = getEffectiveMinuteScale();
      const leftPx = currentMin * minuteScale + channelColWidth;
      const line = document.createElement("div");
      line.className = "now-line-vertical";
      line.style.left = `${leftPx}px`;
      document.getElementById("channelsContainer").appendChild(line);
    } else {
      const headerHeight =
        parseInt(
          getComputedStyle(document.documentElement).getPropertyValue(
            "--header-height",
          ),
        ) || 56;
      const topPx = currentMin * state.currentMinuteHeight + headerHeight;
      const line = document.createElement("div");
      line.className = "now-line";
      line.style.top = `${topPx}px`;
      document.getElementById("channelsContainer").appendChild(line);
    }
  }
}

export function scrollToNow() {
  const viewport = document.getElementById("epgViewport");
  if (!viewport) return;

  const now = new Date();
  const todayStr = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;

  if (state.activeSingleChannelId) {
    const todayWrapper = document.querySelector(
      `[data-date-str="${todayStr}"]`,
    );
    const currentMin = now.getHours() * 60 + now.getMinutes();

    if (state.currentLayout === "horizontal") {
      const minuteScale = getEffectiveMinuteScale();
      viewport.scrollLeft = Math.max(0, currentMin * minuteScale - 300);
      if (todayWrapper) {
        viewport.scrollTop = Math.max(0, todayWrapper.offsetTop - 50);
      }
    } else {
      viewport.scrollTop = Math.max(0, currentMin * state.currentMinuteHeight - 200);
      if (todayWrapper) {
        viewport.scrollLeft = Math.max(0, todayWrapper.offsetLeft - 80);
      }
    }
    return;
  }

  if (state.currentLayout === "horizontal") {
    // Find the earliest starting time among all currently running programs on visible channels
    let earliestStartMin = null;
    if (state.selectedDateStr === todayStr && state.epgData.programmes) {
      const dayStart = new Date(`${todayStr}T00:00:00`);
      const hiddenChannelIds = new Set(
        (state.epgData.channels || [])
          .filter((c) => c.isHidden)
          .map((c) => c.id),
      );
      state.epgData.programmes.forEach((p) => {
        if (hiddenChannelIds.has(p.channel)) return;
        if (p.start <= now && p.stop > now) {
          const sMin =
            p.start <= dayStart
              ? 0
              : p.start.getHours() * 60 + p.start.getMinutes();
          if (earliestStartMin === null || sMin < earliestStartMin) {
            earliestStartMin = sMin;
          }
        }
      });
    }

    const minuteScale = getEffectiveMinuteScale();
    if (earliestStartMin !== null) {
      // Align the beginning of the earliest actual program to the grid start (right after channel column)
      viewport.scrollLeft = Math.max(0, earliestStartMin * minuteScale);
    } else {
      const nowLine = document.querySelector(".now-line-vertical");
      if (nowLine) {
        viewport.scrollLeft = Math.max(0, nowLine.offsetLeft - 350);
      }
    }
  } else {
    const nowLine = document.querySelector(".now-line");
    if (nowLine) {
      viewport.scrollTop = Math.max(0, nowLine.offsetTop - 200);
    }
  }
}

function getEffectiveMinuteScale() {
  if (state.currentLayout === "horizontal") {
    return (
      HORIZONTAL_SCALE_MAP[state.currentStepMin] ||
      state.currentMinuteHeight * 2
    );
  }
  return state.currentMinuteHeight;
}

function setupViewportPan() {
  const viewport = document.getElementById("epgViewport");
  let isPanning = false;
  let hasDragged = false;
  let panStartX = 0;
  let panStartY = 0;
  let panScrollLeft = 0;
  let panScrollTop = 0;
  let panPointerId = null;

  viewport.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;

    // Ha csatorna headert vagy kedvenc csillagot fogunk meg, ne indítsunk pannolást
    if (
      e.target.closest(".channel-header-item") ||
      e.target.closest(".channel-row-header") ||
      e.target.closest(".fav-star") ||
      e.target.closest(".grid-home-btn") ||
      e.target.closest(".channel-all-progs-badge")
    ) {
      return;
    }

    isPanning = false;
    hasDragged = false;
    panStartX = e.clientX;
    panStartY = e.clientY;
    panScrollLeft = viewport.scrollLeft;
    panScrollTop = viewport.scrollTop;
    panPointerId = e.pointerId;
  });

  viewport.addEventListener("pointermove", (e) => {
    if (panPointerId === null || e.pointerId !== panPointerId) return;
    if (e.buttons === 0) {
      endPan(e);
      return;
    }

    const dx = e.clientX - panStartX;
    const dy = e.clientY - panStartY;

    if (!hasDragged && Math.hypot(dx, dy) > 5) {
      hasDragged = true;
      isPanning = true;
      try {
        viewport.setPointerCapture(panPointerId);
      } catch (_err) {}
      viewport.classList.add("is-panning");
    }

    if (isPanning) {
      viewport.scrollLeft = panScrollLeft - dx;
      viewport.scrollTop = panScrollTop - dy;
    }
  });

  const endPan = (e) => {
    if (
      panPointerId === null ||
      (e.pointerId !== undefined && e.pointerId !== panPointerId)
    ) {
      return;
    }

    if (hasDragged) {
      try {
        viewport.releasePointerCapture(panPointerId);
      } catch (_err) {}
      viewport.classList.remove("is-panning");
    }

    isPanning = false;
    panPointerId = null;

    if (hasDragged) {
      setTimeout(() => {
        hasDragged = false;
      }, 100);
    }
  };

  viewport.addEventListener("pointerup", endPan);
  viewport.addEventListener("pointercancel", endPan);

  // Kattintás blokkolása KIZÁRÓLAG akkor, ha valós húzás (drag) történt
  viewport.addEventListener(
    "click",
    (e) => {
      if (hasDragged) {
        e.stopPropagation();
        e.preventDefault();
        hasDragged = false;
      }
    },
    true,
  );
}

export function initEpg() {
  const mainSearchInput = document.getElementById("searchInput");
  const mainClearBtn = document.getElementById("clearSearchBtn");
  attachLiveSearchInput({
    input: mainSearchInput,
    clearBtn: mainClearBtn,
    debounceMs: 300,
    onFilter: () => renderEPG(),
  });

  const viewSelect = document.getElementById("viewSelect");
  if (viewSelect) {
    viewSelect.onchange = async (e) => {
      state.currentStepMin = parseInt(e.target.value, 10);
      state.currentMinuteHeight = VIEW_HEIGHT_MAP[state.currentStepMin] || 2;
      await invoke("set_setting", {
        key: "view_step_min",
        value: String(state.currentStepMin),
      });
      await invoke("set_setting", {
        key: "view_minute_height",
        value: String(state.currentMinuteHeight),
      });
      renderEPG();
    };
  }

  const layoutToggleBtn = document.getElementById("layoutToggleBtn");
  if (layoutToggleBtn) {
    layoutToggleBtn.onclick = async () => {
      state.currentLayout =
        state.currentLayout === "horizontal" ? "vertical" : "horizontal";
      await invoke("set_setting", {
        key: "view_layout",
        value: state.currentLayout,
      });
      updateLayoutButtonUi();
      renderEPG();
    };
  }

  setupViewportPan();

  const gridHomeBtn = document.getElementById("gridHomeBtn");
  if (gridHomeBtn) {
    gridHomeBtn.onclick = (e) => {
      e.stopPropagation();
      if (state.activeSingleChannelId) {
        state.activeSingleChannelId = null;
        renderEPG();
        scrollToNow();
      } else {
        const viewport = document.getElementById("epgViewport");
        if (viewport) {
          if (state.currentLayout === "horizontal") {
            viewport.scrollTop = 0;
          } else {
            viewport.scrollLeft = 0;
          }
        }
        scrollToNow();
      }
    };
  }

  document.getElementById("nowBtn").onclick = () => {
    const now = new Date();
    const todayStr = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
    if (!state.activeSingleChannelId) {
      const dateSelect = document.getElementById("dateSelect");
      const hasToday =
        dateSelect &&
        Array.from(dateSelect.options).some((o) => o.value === todayStr);
      if (hasToday && state.selectedDateStr !== todayStr) {
        state.selectedDateStr = todayStr;
        dateSelect.value = state.selectedDateStr;
        updateDateNavBtns();
        renderEPG();
      }
    }
    scrollToNow();
  };

  setInterval(() => {
    renderNowLine();
  }, 60000);
}
