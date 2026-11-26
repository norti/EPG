import { state, invoke, setFavoritesList } from "./state.js";
import { t, getCurrentLang } from "./i18n.js";
import { SearchUtils } from "../utils/search.js";
import { iconHtml, escapeHtml } from "./ui.js";
import { renderEPG, jumpToEpgAiring } from "./epg.js";
import { openModal, updateModalFavStar } from "./modal.js";
import {
  createCollapseAllButton,
  createCollapsibleGroupElement,
  createLiveSearchBox,
  createMultiSortControls,
  filterAiringsByQuery,
  getAiringStartDate,
  getAiringStopDate,
  getGroupCollapseSummary,
  parseSortLevelsSetting,
  renderAiringGroups,
  setAllGroupsCollapsed,
} from "./airing-list.js";
import { syncWatchlistEpgFavoriteButtons } from "../trakt/trakt-watchlist-epg.js";

const favModalState = {
  sourceFilter: "__ALL_SOURCES__",
  includePast: false,
  query: "",
  sortLevels: null,
  lastPrimarySortKey: null,
  collapsedKeys: new Set(),
  airings: [],
  fetchSeq: 0,
  searchBoxCtrl: null,
  sortControlsCtrl: null,
  collapseAllCtrl: null,
  initialized: false,
};

function pad2(n) {
  return String(n).padStart(2, "0");
}

function formatTimeRange(item) {
  const startDate = getAiringStartDate(item);
  const stopDate = getAiringStopDate(item);
  const startStr = `${pad2(startDate.getHours())}:${pad2(startDate.getMinutes())}`;
  const stopStr = `${pad2(stopDate.getHours())}:${pad2(stopDate.getMinutes())}`;
  return `${startStr} - ${stopStr}`;
}

function ensureSortLevelsInitialized() {
  if (!favModalState.sortLevels) {
    favModalState.sortLevels = parseSortLevelsSetting(
      state.settings?.favorites_sort_levels,
      "date",
    );
  }
}

export function populateFavoritesSourceFilter() {
  const select = document.getElementById("favSourceFilterSelect");
  if (!select) return;

  const prev = favModalState.sourceFilter || "__ALL_SOURCES__";
  select.innerHTML = "";

  const allOpt = document.createElement("option");
  allOpt.value = "__ALL_SOURCES__";
  allOpt.textContent = t("favorites_source_all");
  select.appendChild(allOpt);

  if (state.sources.length > 0) {
    const favChOpt = document.createElement("option");
    favChOpt.value = "__ALL_FAVORITES__";
    favChOpt.textContent = t("all_favorites_source");
    select.appendChild(favChOpt);
  }

  state.sources.forEach((s) => {
    const opt = document.createElement("option");
    opt.value = s.id;
    opt.textContent = `${s.name} [${s.countryCode || "HU"}]`;
    select.appendChild(opt);
  });

  if (Array.from(select.options).some((o) => o.value === prev)) {
    select.value = prev;
  } else {
    favModalState.sourceFilter = "__ALL_SOURCES__";
    select.value = "__ALL_SOURCES__";
  }
}

export async function toggleFavorite(title) {
  const isFav = await invoke("toggle_favorite", { title });
  if (isFav) {
    if (!state.favoritesSet.has(title)) {
      state.favorites.push(title);
      state.favoritesSet.add(title);
    }
  } else {
    setFavoritesList(state.favorites.filter((item) => item !== title));
  }
  updateModalFavStar();
  renderEPG();
  syncWatchlistEpgFavoriteButtons(title, isFav);

  const overlay = document.getElementById("favoritesModalOverlay");
  if (overlay?.classList.contains("active")) {
    if (isFav) {
      await loadAndRenderFavorites({ silent: true, preserveScroll: true });
    } else {
      renderFavoritesList();
    }
  }
  return isFav;
}

function syncFavCollapseAllButton() {
  if (!favModalState.collapseAllCtrl) return;
  const content = document.getElementById("favListContent");
  const { hasGroups, allCollapsed } = getGroupCollapseSummary(
    content,
    favModalState.collapsedKeys,
  );
  favModalState.collapseAllCtrl.updateState(allCollapsed, hasGroups);
}

export function renderFavoritesList() {
  const content = document.getElementById("favListContent");
  const activeDayEl = document.getElementById("favActiveDay");
  const countBadge = document.getElementById("favoritesCountBadge");
  if (!content) return;

  if (favModalState.searchBoxCtrl) {
    favModalState.searchBoxCtrl.refreshLabels();
  }
  if (favModalState.sortControlsCtrl) {
    favModalState.sortControlsCtrl.refreshLabels();
  }
  if (favModalState.collapseAllCtrl) {
    favModalState.collapseAllCtrl.refreshLabels();
  }
  populateFavoritesSourceFilter();

  content.innerHTML = "";
  if (activeDayEl) {
    activeDayEl.classList.add("u-hidden");
    activeDayEl.textContent = "";
  }

  if (state.favorites.length === 0) {
    if (countBadge) countBadge.textContent = "0";
    content.innerHTML = `<p class="empty-list-message">${escapeHtml(t("no_favorites"))}</p>`;
    syncFavCollapseAllButton();
    return;
  }

  ensureSortLevelsInitialized();
  const primaryKey = favModalState.sortLevels[0];
  if (
    favModalState.lastPrimarySortKey &&
    favModalState.lastPrimarySortKey !== primaryKey
  ) {
    favModalState.collapsedKeys.clear();
  }
  favModalState.lastPrimarySortKey = primaryKey;

  const activeAirings = (favModalState.airings || []).filter((a) =>
    state.favoritesSet.has(a.title),
  );
  const matchedTitles = new Set(activeAirings.map((a) => a.title));
  const unmatchedFavorites = state.favorites.filter(
    (favTitle) => !matchedTitles.has(favTitle),
  );

  const filteredAirings = filterAiringsByQuery(
    activeAirings,
    favModalState.query,
  );

  const trimmedQuery = (favModalState.query || "").toLowerCase().trim();
  const progQuery = trimmedQuery ? SearchUtils.prepareQuery(trimmedQuery) : null;
  const filteredUnmatched = progQuery
    ? unmatchedFavorites.filter((favTitle) =>
        SearchUtils.matchPrepared(favTitle, progQuery),
      )
    : unmatchedFavorites;

  const totalVisible = filteredAirings.length + filteredUnmatched.length;
  if (countBadge) {
    countBadge.textContent = String(totalVisible);
  }

  if (totalVisible === 0) {
    content.innerHTML = `<p class="empty-list-message">${escapeHtml(t("no_channels_match_filter"))}</p>`;
    syncFavCollapseAllButton();
    return;
  }

  const frag = document.createDocumentFragment();

  if (filteredAirings.length > 0) {
    renderAiringGroups({
      container: content,
      activeHeaderEl: activeDayEl,
      items: filteredAirings,
      levels: favModalState.sortLevels,
      lang: getCurrentLang(),
      appendFragment: frag,
      collapsedKeys: favModalState.collapsedKeys,
      onCollapseChange: syncFavCollapseAllButton,
      rowOptions: {
        showRemoveFavorite: true,
        onJump: (item) => jumpToEpgAiring(item),
        onOpenModal: (item) => {
          const channel = {
            id: item.channelId,
            name: item.channelName,
            sourceId: item.sourceId,
            sourceName: item.sourceName,
            countryCode: item.countryCode,
          };
          openModal(item.title, formatTimeRange(item), item.desc, channel, {
            airing: {
              sourceId: item.sourceId || "",
              channelId: item.channelId || "",
              startDate: getAiringStartDate(item),
              stopDate: getAiringStopDate(item),
              channelName: item.channelName || "",
              sourceName: item.sourceName || "",
            },
          });
        },
        onRemoveFavorite: (item) => toggleFavorite(item.title),
      },
    });
  }

  if (filteredUnmatched.length > 0) {
    const unscheduledHeading = t("favorites_unscheduled");
    const { groupDiv, itemsWrap } = createCollapsibleGroupElement({
      groupKey: "unscheduled",
      label: unscheduledHeading,
      count: filteredUnmatched.length,
      collapsedKeys: favModalState.collapsedKeys,
      onToggleGroup: syncFavCollapseAllButton,
    });

    filteredUnmatched.forEach((favTitle) => {
      const row = document.createElement("div");
      row.className = "fav-item-row";
      row.innerHTML = `
        <button type="button" class="airing-row-title fav-item-title" title="${escapeHtml(favTitle)}">${escapeHtml(favTitle)}</button>
        <button type="button" class="btn btn-red btn-icon-only fav-remove-btn" title="${escapeHtml(t("trakt_btn_fav_remove"))}">${iconHtml("trash", 14)}</button>
      `;
      row.querySelector(".airing-row-title").onclick = () =>
        openModal(favTitle, "", "", null);
      row.querySelector(".fav-remove-btn").onclick = () =>
        toggleFavorite(favTitle);
      itemsWrap.appendChild(row);
    });

    frag.appendChild(groupDiv);
  }

  content.appendChild(frag);
  syncFavCollapseAllButton();
}

export async function loadAndRenderFavorites(options = {}) {
  const seq = ++favModalState.fetchSeq;
  const content = document.getElementById("favListContent");
  if (!content) return;

  if (state.favorites.length === 0) {
    favModalState.airings = [];
    renderFavoritesList();
    return;
  }

  const prevScrollTop = options.preserveScroll ? content.scrollTop : 0;
  if (!options.silent) {
    content.innerHTML = `<div class="trakt-loading"><span class="spinner-small"></span> ${escapeHtml(t("status_loading"))}</div>`;
  }

  try {
    const airings = await invoke("get_favorite_programmes_airings", {
      sourceId: favModalState.sourceFilter,
      includePast: favModalState.includePast,
    });
    if (seq !== favModalState.fetchSeq) return;
    favModalState.airings = Array.isArray(airings) ? airings : [];
    renderFavoritesList();
    if (options.preserveScroll && prevScrollTop > 0) {
      content.scrollTop = prevScrollTop;
    }
  } catch (err) {
    if (seq !== favModalState.fetchSeq) return;
    content.innerHTML = `<div class="trakt-error-box">${escapeHtml(String(err))}</div>`;
  }
}

export function openFavoritesModal() {
  const overlay = document.getElementById("favoritesModalOverlay");
  if (!overlay) return;
  ensureSortLevelsInitialized();
  if (favModalState.sortControlsCtrl) {
    favModalState.sortControlsCtrl.setLevels(favModalState.sortLevels);
  }
  populateFavoritesSourceFilter();
  overlay.classList.add("active");
  loadAndRenderFavorites();
}

export function closeFavoritesModal() {
  const overlay = document.getElementById("favoritesModalOverlay");
  if (overlay) overlay.classList.remove("active");
}

export function refreshFavoritesModalIfOpen() {
  const overlay = document.getElementById("favoritesModalOverlay");
  if (overlay?.classList.contains("active")) {
    loadAndRenderFavorites({ silent: true, preserveScroll: true });
  }
}

export function initFavorites() {
  if (favModalState.initialized) return;
  favModalState.initialized = true;

  favModalState.sortLevels = parseSortLevelsSetting(
    state.settings?.favorites_sort_levels,
    "date",
  );

  const searchMount = document.getElementById("favSearchBoxMount");
  if (searchMount) {
    favModalState.searchBoxCtrl = createLiveSearchBox({
      inputId: "favSearchInput",
      clearBtnId: "favClearSearchBtn",
      initialValue: favModalState.query,
      onFilter: (q) => {
        favModalState.query = q;
        renderFavoritesList();
      },
    });
    searchMount.appendChild(favModalState.searchBoxCtrl.element);
  }

  const collapseMount = document.getElementById("favCollapseAllMount");
  if (collapseMount) {
    favModalState.collapseAllCtrl = createCollapseAllButton({
      onToggle: (allCollapsed) => {
        const content = document.getElementById("favListContent");
        setAllGroupsCollapsed(
          content,
          favModalState.collapsedKeys,
          !allCollapsed,
        );
        syncFavCollapseAllButton();
      },
    });
    collapseMount.appendChild(favModalState.collapseAllCtrl.element);
  }

  const sortMount = document.getElementById("favSortControlsMount");
  if (sortMount) {
    favModalState.sortControlsCtrl = createMultiSortControls({
      initialLevels: favModalState.sortLevels,
      defaultKey: "date",
      onChange: (levels) => {
        favModalState.sortLevels = levels;
        const serialized = JSON.stringify(levels);
        if (state.settings) state.settings.favorites_sort_levels = serialized;
        invoke("set_setting", {
          key: "favorites_sort_levels",
          value: serialized,
        }).catch(() => {});
        renderFavoritesList();
      },
    });
    sortMount.appendChild(favModalState.sortControlsCtrl.element);
  }

  const favBtn = document.getElementById("favListBtn");
  if (favBtn) {
    favBtn.onclick = () => openFavoritesModal();
  }

  const closeBtn = document.getElementById("favoritesModalClose");
  if (closeBtn) {
    closeBtn.onclick = () => closeFavoritesModal();
  }

  const sourceSelect = document.getElementById("favSourceFilterSelect");
  if (sourceSelect) {
    sourceSelect.onchange = () => {
      favModalState.sourceFilter = sourceSelect.value;
      loadAndRenderFavorites();
    };
  }

  const pastCheck = document.getElementById("favIncludePastCheck");
  if (pastCheck) {
    pastCheck.checked = favModalState.includePast;
    pastCheck.onchange = () => {
      favModalState.includePast = pastCheck.checked;
      loadAndRenderFavorites();
    };
  }
}
