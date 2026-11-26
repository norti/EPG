import { LOCALE_COMPARE_OPTIONS } from "./state.js";
import { t, getCurrentLang } from "./i18n.js";
import { SearchUtils } from "../utils/search.js";
import {
  iconHtml,
  escapeHtml,
  attachLiveSearchInput,
  createLiveSearchBox,
} from "./ui.js";
import { parseEPGDate, formatDateLabel, isFavoriteTitle } from "./epg.js";
import { TraktDates } from "../trakt/trakt-dates.js";

export { attachLiveSearchInput, createLiveSearchBox };

export const SORT_KEYS = Object.freeze([
  "date",
  "title",
  "channel",
  "source",
]);

const SORT_LABEL_KEYS = Object.freeze({
  date: "sort_by_date",
  title: "sort_by_title",
  channel: "sort_by_channel",
  source: "sort_by_source",
});

function pad2(n) {
  return String(n).padStart(2, "0");
}

export function normalizeSortLevels(levels, defaultKey = "date") {
  const fallback = SORT_KEYS.includes(defaultKey) ? defaultKey : "date";
  const out = [];
  if (Array.isArray(levels)) {
    for (const rawKey of levels) {
      const key = rawKey === "source_channel" ? "source" : rawKey;
      if (SORT_KEYS.includes(key) && !out.includes(key)) {
        out.push(key);
        if (out.length >= 3) break;
      }
    }
  }
  if (out.length === 0) {
    out.push(fallback);
  }
  return out;
}

export function parseSortLevelsSetting(rawValue, defaultKey = "date") {
  if (Array.isArray(rawValue)) {
    return normalizeSortLevels(rawValue, defaultKey);
  }
  if (typeof rawValue === "string" && rawValue.trim()) {
    try {
      const parsed = JSON.parse(rawValue);
      return normalizeSortLevels(parsed, defaultKey);
    } catch (_) {
      // Ignore malformed setting and fall back to default
    }
  }
  return normalizeSortLevels([], defaultKey);
}

export function getAiringStartDate(item) {
  if (!item) return new Date(0);
  if (item.startDate instanceof Date && !isNaN(item.startDate.getTime())) {
    return item.startDate;
  }
  if (item._startDate instanceof Date && !isNaN(item._startDate.getTime())) {
    return item._startDate;
  }
  let d;
  if (item.start) {
    d = parseEPGDate(item.start, item.timeOffset || 0);
  } else if (typeof item.startTs === "number") {
    d = new Date((item.startTs + (item.timeOffset || 0) * 3600) * 1000);
  } else {
    d = new Date(0);
  }
  item._startDate = d;
  return d;
}

export function getAiringStopDate(item) {
  if (!item) return new Date(0);
  if (item.stopDate instanceof Date && !isNaN(item.stopDate.getTime())) {
    return item.stopDate;
  }
  if (item._stopDate instanceof Date && !isNaN(item._stopDate.getTime())) {
    return item._stopDate;
  }
  let d;
  if (item.stop) {
    d = parseEPGDate(item.stop, item.timeOffset || 0);
  } else if (typeof item.stopTs === "number") {
    d = new Date((item.stopTs + (item.timeOffset || 0) * 3600) * 1000);
  } else {
    d = getAiringStartDate(item);
  }
  item._stopDate = d;
  return d;
}

function getAiringDayNum(item) {
  const d = getAiringStartDate(item);
  return d.getFullYear() * 10000 + (d.getMonth() + 1) * 100 + d.getDate();
}

function compareByCriterion(a, b, key, locale) {
  if (key === "date") {
    return getAiringDayNum(a) - getAiringDayNum(b);
  }
  if (key === "title") {
    return (a.title || "").localeCompare(
      b.title || "",
      locale,
      LOCALE_COMPARE_OPTIONS,
    );
  }
  if (key === "channel") {
    return (a.channelName || "").localeCompare(
      b.channelName || "",
      locale,
      LOCALE_COMPARE_OPTIONS,
    );
  }
  if (key === "source") {
    return (a.sourceName || "").localeCompare(
      b.sourceName || "",
      locale,
      LOCALE_COMPARE_OPTIONS,
    );
  }
  return 0;
}

export function sortAirings(items, levels, lang = getCurrentLang()) {
  const activeLevels = normalizeSortLevels(levels);
  const locale = lang === "english" ? "en" : "hu";
  const tieBreakers = ["title", "source", "channel"];

  return [...(items || [])].sort((a, b) => {
    for (const key of activeLevels) {
      const cmp = compareByCriterion(a, b, key, locale);
      if (cmp !== 0) return cmp;
    }
    const timeCmp =
      getAiringStartDate(a).getTime() - getAiringStartDate(b).getTime();
    if (timeCmp !== 0) return timeCmp;
    for (const tb of tieBreakers) {
      if (activeLevels.includes(tb)) continue;
      const cmp = compareByCriterion(a, b, tb, locale);
      if (cmp !== 0) return cmp;
    }
    return 0;
  });
}

export function getGroupInfo(item, primaryKey) {
  if (primaryKey === "title") {
    const norm = SearchUtils.normalizeText(item.title || "").replace(
      /^[^\p{L}\p{N}]+/u,
      "",
    );
    const first = norm.charAt(0).toUpperCase();
    let label = "#";
    if (/^[A-Z]$/.test(first)) {
      label = first;
    } else if (/^[0-9]$/.test(first)) {
      label = "0-9";
    }
    return { key: `title:${label}`, label };
  }

  if (primaryKey === "channel") {
    const ch = item.channelName || item.channelId || "";
    return { key: `ch:${ch.toLowerCase()}`, label: ch };
  }

  if (primaryKey === "source") {
    const src = item.sourceName || item.sourceId || "";
    return {
      key: `src:${src.toLowerCase()}`,
      label: src,
    };
  }

  const d = getAiringStartDate(item);
  const ymd = `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
  return { key: `date:${ymd}`, label: formatDateLabel(ymd) };
}

export function groupAirings(sortedItems, primaryKey) {
  const groups = [];
  const groupMap = new Map();

  for (const item of sortedItems || []) {
    const info = getGroupInfo(item, primaryKey);
    let group = groupMap.get(info.key);
    if (!group) {
      group = { key: info.key, label: info.label, items: [] };
      groupMap.set(info.key, group);
      groups.push(group);
    }
    group.items.push(item);
  }

  return groups;
}

export function filterAiringsByQuery(items, query) {
  const trimmed = (query || "").toLowerCase().trim();
  if (!trimmed) return items || [];

  const progQuery = SearchUtils.prepareQuery(trimmed);
  const metaQuery = SearchUtils.prepareQuery(trimmed, { minFuzzyLength: 6 });

  return (items || []).filter((item) => {
    if (SearchUtils.matchPrepared(item.title || "", progQuery)) return true;
    if (
      item.watchlistTitle &&
      SearchUtils.matchPrepared(item.watchlistTitle, progQuery)
    ) {
      return true;
    }
    if (
      item.watchlistTitleHu &&
      SearchUtils.matchPrepared(item.watchlistTitleHu, progQuery)
    ) {
      return true;
    }
    if (
      Array.isArray(item.aliases) &&
      item.aliases.some((a) => SearchUtils.matchPrepared(a, progQuery))
    ) {
      return true;
    }
    if (SearchUtils.matchPrepared(item.channelName || "", metaQuery)) {
      return true;
    }
    if (SearchUtils.matchPrepared(item.sourceName || "", metaQuery)) {
      return true;
    }
    return false;
  });
}

export function createMultiSortControls({
  initialLevels,
  defaultKey = "date",
  onChange,
}) {
  let levels = normalizeSortLevels(initialLevels, defaultKey);

  const bar = document.createElement("div");
  bar.className = "multi-sort-bar";

  const selects = [];
  const labels = [];
  const labelKeys = ["sort_level_1", "sort_level_2", "sort_level_3"];

  for (let i = 0; i < 3; i++) {
    const group = document.createElement("div");
    group.className = "multi-sort-level";

    const lbl = document.createElement("label");
    lbl.className = "settings-label multi-sort-label";
    lbl.textContent = t(labelKeys[i]);
    lbl.setAttribute("data-i18n", labelKeys[i]);

    const sel = document.createElement("select");
    sel.className = "multi-sort-select";
    lbl.htmlFor = "";

    group.appendChild(lbl);
    group.appendChild(sel);
    bar.appendChild(group);

    labels.push(lbl);
    selects.push(sel);
  }

  const populateSelects = () => {
    const l1 = levels[0];
    const l2 = levels[1] || "";
    const l3 = levels[2] || "";

    // Level 1: all 4 options
    selects[0].innerHTML = "";
    for (const key of SORT_KEYS) {
      const opt = document.createElement("option");
      opt.value = key;
      opt.textContent = t(SORT_LABEL_KEYS[key]);
      selects[0].appendChild(opt);
    }
    selects[0].value = l1;

    // Level 2: None + remaining options
    selects[1].innerHTML = "";
    const none2 = document.createElement("option");
    none2.value = "";
    none2.textContent = t("sort_none");
    selects[1].appendChild(none2);
    for (const key of SORT_KEYS) {
      if (key === l1) continue;
      const opt = document.createElement("option");
      opt.value = key;
      opt.textContent = t(SORT_LABEL_KEYS[key]);
      selects[1].appendChild(opt);
    }
    selects[1].value = l2;

    // Level 3: None + remaining options (disabled if Level 2 is empty)
    selects[2].innerHTML = "";
    const none3 = document.createElement("option");
    none3.value = "";
    none3.textContent = t("sort_none");
    selects[2].appendChild(none3);
    for (const key of SORT_KEYS) {
      if (key === l1 || key === l2) continue;
      const opt = document.createElement("option");
      opt.value = key;
      opt.textContent = t(SORT_LABEL_KEYS[key]);
      selects[2].appendChild(opt);
    }
    selects[2].disabled = !l2;
    selects[2].value = l2 ? l3 : "";
  };

  selects[0].onchange = () => {
    const next1 = selects[0].value;
    let next2 = levels[1] || "";
    let next3 = levels[2] || "";
    if (next2 === next1) {
      next2 = next3 && next3 !== next1 ? next3 : "";
      next3 = "";
    } else if (next3 === next1) {
      next3 = "";
    }
    levels = normalizeSortLevels(
      [next1, next2, next3].filter(Boolean),
      defaultKey,
    );
    populateSelects();
    if (typeof onChange === "function") onChange([...levels]);
  };

  selects[1].onchange = () => {
    const next1 = levels[0];
    const next2 = selects[1].value;
    let next3 = next2 ? levels[2] || "" : "";
    if (next3 === next2 || next3 === next1) {
      next3 = "";
    }
    levels = normalizeSortLevels(
      [next1, next2, next3].filter(Boolean),
      defaultKey,
    );
    populateSelects();
    if (typeof onChange === "function") onChange([...levels]);
  };

  selects[2].onchange = () => {
    const next1 = levels[0];
    const next2 = levels[1] || "";
    const next3 = next2 ? selects[2].value : "";
    levels = normalizeSortLevels(
      [next1, next2, next3].filter(Boolean),
      defaultKey,
    );
    populateSelects();
    if (typeof onChange === "function") onChange([...levels]);
  };

  populateSelects();

  return {
    element: bar,
    getLevels: () => [...levels],
    setLevels: (nextLevels) => {
      levels = normalizeSortLevels(nextLevels, defaultKey);
      populateSelects();
    },
    refreshLabels: () => {
      labels.forEach((lbl, idx) => {
        lbl.textContent = t(labelKeys[idx]);
      });
      populateSelects();
    },
  };
}

export function formatAiringDateMeta(startDate, lang = getCurrentLang()) {
  const locale = lang === "english" ? "en-US" : "hu-HU";
  const fullDateStr = startDate.toLocaleDateString(locale, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });

  const todayMid = new Date();
  todayMid.setHours(0, 0, 0, 0);
  const targetMid = new Date(
    startDate.getFullYear(),
    startDate.getMonth(),
    startDate.getDate(),
  );
  const diffDays = Math.round((targetMid - todayMid) / 86400000);
  const daysArr = t("days");
  let dayPillText =
    Array.isArray(daysArr) && daysArr[startDate.getDay()]
      ? daysArr[startDate.getDay()]
      : startDate.toLocaleDateString(locale, { weekday: "long" });
  let dayPillClass = "trakt-airing-day-pill";
  if (diffDays === 0) {
    dayPillText = t("day_today");
    dayPillClass += " is-today";
  } else if (diffDays === 1) {
    dayPillText = t("day_tomorrow");
    dayPillClass += " is-tomorrow";
  } else if (diffDays === -1) {
    dayPillText = t("day_yesterday");
  }

  const startTimeStr = `${pad2(startDate.getHours())}:${pad2(startDate.getMinutes())}`;
  return { fullDateStr, dayPillText, dayPillClass, startTimeStr };
}

export function renderAiringRow(item, options = {}) {
  const lang = options.lang || getCurrentLang();
  const startDate = getAiringStartDate(item);
  const { fullDateStr, dayPillText, dayPillClass, startTimeStr } =
    formatAiringDateMeta(startDate, lang);

  const row = document.createElement("div");
  row.className = "airing-list-row";
  if (options.showRemoveWatchlist) {
    row.classList.add("has-wl-remove");
  }

  const isFav = isFavoriteTitle(item.title);
  const favBtnHtml = options.showFavoriteToggle
    ? `<button type="button" class="trakt-airing-icon-btn trakt-airing-fav-btn ${isFav ? "active" : ""}" data-prog-title="${escapeHtml(item.title)}" title="${escapeHtml(isFav ? t("trakt_btn_fav_remove") : t("trakt_btn_fav_add"))}">${iconHtml(isFav ? "star-filled" : "star", 15)}</button>`
    : "";

  const removeFavHtml = options.showRemoveFavorite
    ? `<button type="button" class="btn btn-red btn-icon-only fav-remove-btn" title="${escapeHtml(t("trakt_btn_fav_remove"))}">${iconHtml("trash", 14)}</button>`
    : "";

  const removeWlHtml = options.showRemoveWatchlist
    ? `<button type="button" class="btn btn-red btn-icon-only trakt-airing-wl-remove-btn" title="${escapeHtml(t("trakt_btn_remove_watchlist"))}">${iconHtml("trakt-watchlist-remove", 15)}</button>`
    : "";

  row.innerHTML = `
    <button type="button" class="trakt-airing-icon-btn trakt-airing-jump-btn" title="${escapeHtml(t("trakt_btn_jump_epg"))}">
      ${iconHtml("trakt-jump-epg", 15)}
    </button>
    ${favBtnHtml}
    <button type="button" class="airing-row-title" title="${escapeHtml(item.title)}">${escapeHtml(item.title)}</button>
    <span class="channel-source-badge airing-row-source" title="${escapeHtml(item.sourceName || "")}">${escapeHtml(item.sourceName || "")}</span>
    <span class="channel-badge airing-row-channel" title="${escapeHtml(item.channelName || "")}">${escapeHtml(item.channelName || "")}</span>
    <span class="trakt-airing-date airing-row-date" title="${escapeHtml(fullDateStr)}">${escapeHtml(fullDateStr)}</span>
    <span class="${dayPillClass} airing-row-day" title="${escapeHtml(dayPillText)}">${escapeHtml(dayPillText)}</span>
    <span class="start-badge trakt-airing-time-pill airing-row-time">${escapeHtml(startTimeStr)}</span>
    ${removeFavHtml}
    ${removeWlHtml}
  `;

  const jumpBtn = row.querySelector(".trakt-airing-jump-btn");
  if (jumpBtn && typeof options.onJump === "function") {
    jumpBtn.onclick = (e) => {
      e.stopPropagation();
      options.onJump(item);
    };
  }

  const titleBtn = row.querySelector(".airing-row-title");
  if (titleBtn && typeof options.onOpenModal === "function") {
    titleBtn.onclick = (e) => {
      e.stopPropagation();
      options.onOpenModal(item);
    };
  }

  const favBtn = row.querySelector(".trakt-airing-fav-btn");
  if (favBtn && typeof options.onToggleFavorite === "function") {
    favBtn.onclick = (e) => {
      e.stopPropagation();
      options.onToggleFavorite(item, favBtn);
    };
  }

  const removeFavBtn = row.querySelector(".fav-remove-btn");
  if (removeFavBtn && typeof options.onRemoveFavorite === "function") {
    removeFavBtn.onclick = (e) => {
      e.stopPropagation();
      options.onRemoveFavorite(item, removeFavBtn);
    };
  }

  const removeWlBtn = row.querySelector(".trakt-airing-wl-remove-btn");
  if (removeWlBtn && typeof options.onRemoveWatchlist === "function") {
    removeWlBtn.onclick = (e) => {
      e.stopPropagation();
      options.onRemoveWatchlist(item, removeWlBtn);
    };
  }

  return row;
}

export function createCollapseAllButton({ onToggle }) {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "btn airing-collapse-all-btn btn-icon-only";
  btn.disabled = true;

  let currentAllCollapsed = false;
  let currentHasGroups = false;

  const updateState = (allCollapsed, hasGroups = true) => {
    currentAllCollapsed = !!allCollapsed;
    currentHasGroups = !!hasGroups;
    btn.disabled = !currentHasGroups;
    const label = currentAllCollapsed
      ? t("airing_expand_all")
      : t("airing_collapse_all");
    btn.title = label;
    btn.setAttribute("aria-label", label);
    btn.innerHTML = iconHtml(currentAllCollapsed ? "fold-down" : "fold-up", 16);
  };

  btn.onclick = () => {
    if (btn.disabled) return;
    if (typeof onToggle === "function") {
      onToggle(currentAllCollapsed);
    }
  };

  updateState(false, false);

  return {
    element: btn,
    updateState,
    refreshLabels: () => updateState(currentAllCollapsed, currentHasGroups),
  };
}

export function createCollapsibleGroupElement({
  groupKey,
  label,
  count,
  collapsedKeys,
  onToggleGroup,
}) {
  const isCollapsed = !!(collapsedKeys && collapsedKeys.has(groupKey));

  const groupDiv = document.createElement("div");
  groupDiv.className = `fav-day-group${isCollapsed ? " is-collapsed" : ""}`;
  groupDiv.dataset.groupKey = groupKey;
  groupDiv.dataset.dayHeading = label;

  const headerBtn = document.createElement("button");
  headerBtn.type = "button";
  headerBtn.className = "fav-day-header-btn";
  headerBtn.setAttribute("aria-expanded", String(!isCollapsed));
  headerBtn.innerHTML = `
    <span class="fav-day-chevron">${iconHtml(isCollapsed ? "chevron-right" : "chevron-down", 14)}</span>
    <span class="fav-day-header-label">${escapeHtml(label)}</span>
    <span class="fav-day-header-count">${count}</span>
  `;

  const itemsWrap = document.createElement("div");
  itemsWrap.className = "fav-day-items";

  headerBtn.onclick = () => {
    const nowCollapsed = !groupDiv.classList.contains("is-collapsed");
    groupDiv.classList.toggle("is-collapsed", nowCollapsed);
    headerBtn.setAttribute("aria-expanded", String(!nowCollapsed));
    const chev = headerBtn.querySelector(".fav-day-chevron");
    if (chev) {
      chev.innerHTML = iconHtml(
        nowCollapsed ? "chevron-right" : "chevron-down",
        14,
      );
    }
    if (collapsedKeys) {
      if (nowCollapsed) collapsedKeys.add(groupKey);
      else collapsedKeys.delete(groupKey);
    }
    if (typeof onToggleGroup === "function") {
      onToggleGroup(groupKey, nowCollapsed);
    }
  };

  groupDiv.appendChild(headerBtn);
  groupDiv.appendChild(itemsWrap);

  return { groupDiv, itemsWrap };
}

export function getGroupCollapseSummary(container, collapsedKeys) {
  if (!container) return { hasGroups: false, allCollapsed: false };
  const groups = Array.from(container.querySelectorAll(".fav-day-group"));
  if (groups.length === 0) return { hasGroups: false, allCollapsed: false };
  const allCollapsed = groups.every((g) => {
    const key = g.dataset.groupKey;
    return key && collapsedKeys ? collapsedKeys.has(key) : g.classList.contains("is-collapsed");
  });
  return { hasGroups: true, allCollapsed };
}

export function setAllGroupsCollapsed(container, collapsedKeys, collapse) {
  if (!container) return;
  const groups = Array.from(container.querySelectorAll(".fav-day-group"));
  for (const groupDiv of groups) {
    const key = groupDiv.dataset.groupKey;
    groupDiv.classList.toggle("is-collapsed", !!collapse);
    const headerBtn = groupDiv.querySelector(".fav-day-header-btn");
    if (headerBtn) {
      headerBtn.setAttribute("aria-expanded", String(!collapse));
      const chev = headerBtn.querySelector(".fav-day-chevron");
      if (chev) {
        chev.innerHTML = iconHtml(
          collapse ? "chevron-right" : "chevron-down",
          14,
        );
      }
    }
    if (collapsedKeys && key) {
      if (collapse) collapsedKeys.add(key);
      else collapsedKeys.delete(key);
    }
  }
}

export function renderAiringGroups({
  container,
  activeHeaderEl,
  items,
  levels,
  lang = getCurrentLang(),
  rowOptions = {},
  appendFragment = null,
  collapsedKeys = null,
  onCollapseChange = null,
}) {
  if (!container) return;
  const activeLevels = normalizeSortLevels(levels);
  const sorted = sortAirings(items, activeLevels, lang);
  const groups = groupAirings(sorted, activeLevels[0]);

  const frag = appendFragment || document.createDocumentFragment();

  for (const group of groups) {
    const { groupDiv, itemsWrap } = createCollapsibleGroupElement({
      groupKey: group.key,
      label: group.label,
      count: group.items.length,
      collapsedKeys,
      onToggleGroup: () => {
        if (typeof onCollapseChange === "function") {
          onCollapseChange();
        }
      },
    });

    for (const item of group.items) {
      itemsWrap.appendChild(renderAiringRow(item, { ...rowOptions, lang }));
    }

    frag.appendChild(groupDiv);
  }

  if (!appendFragment) {
    container.innerHTML = "";
    container.appendChild(frag);
  }

  if (activeHeaderEl && typeof TraktDates.attachStickyDayHeader === "function") {
    TraktDates.attachStickyDayHeader(container, activeHeaderEl, {
      groupSelector: ".fav-day-group",
    });
  }
}
