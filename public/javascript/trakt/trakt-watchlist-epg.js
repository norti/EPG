import { state, invoke } from "../application/state.js";
import { t, getCurrentLang } from "../application/i18n.js";
import { iconHtml, escapeHtml } from "../application/ui.js";
import { jumpToEpgAiring } from "../application/epg.js";
import { toggleFavorite } from "../application/favorites.js";
import { openModal } from "../application/modal.js";
import {
  createCollapseAllButton,
  createLiveSearchBox,
  createMultiSortControls,
  filterAiringsByQuery,
  getAiringStartDate,
  getAiringStopDate,
  getGroupCollapseSummary,
  parseSortLevelsSetting,
  renderAiringGroups,
  setAllGroupsCollapsed,
} from "../application/airing-list.js";
import { showTraktError } from "./trakt-ui.js";

const wlEpgState = {
  filter: "all",
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
};

function ensureSortLevelsInitialized() {
  if (!wlEpgState.sortLevels) {
    wlEpgState.sortLevels = parseSortLevelsSetting(
      state.settings?.watchlist_epg_sort_levels,
      "title",
    );
  }
}

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

export function syncWatchlistEpgFavoriteButtons(progTitle, isFav) {
  if (!progTitle) return;
  document.querySelectorAll(".trakt-airing-fav-btn").forEach((btn) => {
    if (btn.dataset.progTitle === progTitle) {
      btn.classList.toggle("active", !!isFav);
      btn.title = isFav ? t("trakt_btn_fav_remove") : t("trakt_btn_fav_add");
      btn.innerHTML = iconHtml(isFav ? "star-filled" : "star", 15);
    }
  });
}

function syncWlEpgCollapseAllButton() {
  if (!wlEpgState.collapseAllCtrl) return;
  const listEl = document.getElementById("traktWatchlistEpgList");
  const { hasGroups, allCollapsed } = getGroupCollapseSummary(
    listEl,
    wlEpgState.collapsedKeys,
  );
  wlEpgState.collapseAllCtrl.updateState(allCollapsed, hasGroups);
}

function renderWatchlistEpgList() {
  const listEl = document.getElementById("traktWatchlistEpgList");
  const activeGroupEl = document.getElementById("traktWlEpgActiveGroup");
  const countEl = document.getElementById("traktWlEpgCount");
  if (!listEl || !listEl.isConnected) return;

  if (activeGroupEl) {
    activeGroupEl.classList.add("u-hidden");
    activeGroupEl.textContent = "";
  }

  const primaryKey = wlEpgState.sortLevels[0];
  if (
    wlEpgState.lastPrimarySortKey &&
    wlEpgState.lastPrimarySortKey !== primaryKey
  ) {
    wlEpgState.collapsedKeys.clear();
  }
  wlEpgState.lastPrimarySortKey = primaryKey;

  const typeFiltered = (wlEpgState.airings || []).filter((item) => {
    if (wlEpgState.filter === "shows" && item.mediaType !== "show") return false;
    if (wlEpgState.filter === "movies" && item.mediaType !== "movie") return false;
    return true;
  });

  const filtered = filterAiringsByQuery(typeFiltered, wlEpgState.query);

  if (countEl) {
    countEl.textContent = String(filtered.length);
  }

  if (filtered.length === 0) {
    listEl.innerHTML = `<div class="trakt-empty-msg">${escapeHtml(t("trakt_watchlist_epg_empty"))}</div>`;
    syncWlEpgCollapseAllButton();
    return;
  }

  renderAiringGroups({
    container: listEl,
    activeHeaderEl: activeGroupEl,
    items: filtered,
    levels: wlEpgState.sortLevels,
    lang: getCurrentLang(),
    collapsedKeys: wlEpgState.collapsedKeys,
    onCollapseChange: syncWlEpgCollapseAllButton,
    rowOptions: {
      showFavoriteToggle: true,
      showRemoveWatchlist: true,
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
          fallbackTmdbId: item.tmdbId || null,
          fallbackMediaType: item.mediaType === "movie" ? "movie" : "tv",
        });
      },
      onToggleFavorite: async (item, btn) => {
        if (!item.title) return;
        btn.disabled = true;
        try {
          const newFav = await toggleFavorite(item.title);
          syncWatchlistEpgFavoriteButtons(item.title, newFav);
        } finally {
          if (btn.isConnected) btn.disabled = false;
        }
      },
      onRemoveWatchlist: async (item, btn) => {
        btn.disabled = true;
        try {
          await invoke("trakt_watchlist_remove", {
            traktId: item.traktId,
            mediaType: item.mediaType,
          });
          wlEpgState.airings = wlEpgState.airings.filter(
            (x) => !(x.traktId === item.traktId && x.mediaType === item.mediaType),
          );
          renderWatchlistEpgList();
        } catch (err) {
          if (btn.isConnected) btn.disabled = false;
          showTraktError(err);
        }
      },
    },
  });
  syncWlEpgCollapseAllButton();
}

async function fetchAndRenderWatchlistEpg(options = {}) {
  const seq = ++wlEpgState.fetchSeq;
  const listEl = document.getElementById("traktWatchlistEpgList");
  if (!listEl || !listEl.isConnected) return;

  const prevScrollTop = options.preserveScroll ? listEl.scrollTop : 0;
  if (!options.silent) {
    listEl.innerHTML = `<div class="trakt-loading"><span class="spinner-small"></span> ${escapeHtml(t("trakt_loading"))}</div>`;
  }

  try {
    const airings = await invoke("trakt_get_watchlist_epg_airings", {
      includePast: wlEpgState.includePast,
    });
    if (seq !== wlEpgState.fetchSeq || !listEl.isConnected) return;
    wlEpgState.airings = Array.isArray(airings) ? airings : [];
    renderWatchlistEpgList();
    if (options.preserveScroll && prevScrollTop > 0) {
      listEl.scrollTop = prevScrollTop;
    }
  } catch (err) {
    if (seq !== wlEpgState.fetchSeq || !listEl.isConnected) return;
    listEl.innerHTML = `<div class="trakt-error-box">${escapeHtml(String(err))}</div>`;
  }
}

export async function renderWatchlistEpgView(viewport, options = {}) {
  if (!viewport) return;
  ensureSortLevelsInitialized();

  if (wlEpgState.searchBoxCtrl) {
    wlEpgState.searchBoxCtrl.destroy();
    wlEpgState.searchBoxCtrl = null;
  }

  const initialFilter = wlEpgState.filter || "all";

  viewport.innerHTML = `
    <div class="trakt-subview">
      <div class="trakt-subview-toolbar trakt-wl-epg-toolbar">
        <div class="trakt-wl-epg-toolbar-top">
          <h2 class="trakt-subview-title">${iconHtml("device-tv", 20)} ${escapeHtml(t("trakt_watchlist_epg_title"))}</h2>
          <span class="trakt-cal-active-day u-hidden" id="traktWlEpgActiveGroup"></span>
          <span class="trakt-panel-count" id="traktWlEpgCount">0</span>
          <div class="trakt-filter-pills">
            <button type="button" class="btn trakt-pill ${initialFilter === "all" ? "active" : ""}" data-wl-epg-filter="all">${escapeHtml(t("filter_all"))}</button>
            <button type="button" class="btn trakt-pill ${initialFilter === "shows" ? "active" : ""}" data-wl-epg-filter="shows">${escapeHtml(t("trakt_filter_shows"))}</button>
            <button type="button" class="btn trakt-pill ${initialFilter === "movies" ? "active" : ""}" data-wl-epg-filter="movies">${escapeHtml(t("trakt_filter_movies"))}</button>
          </div>
          <div id="traktWlEpgSearchMount"></div>
        </div>
        <div class="trakt-wl-epg-toolbar-bottom">
          <div class="trakt-wl-epg-past-group">
            <span id="traktWlEpgCollapseAllMount"></span>
            <label class="logos-checkbox-label">
              <input type="checkbox" id="traktWlEpgIncludePastCheck" ${wlEpgState.includePast ? "checked" : ""} />
              <span>${escapeHtml(t("airing_include_past"))}</span>
            </label>
          </div>
          <div id="traktWlEpgSortMount"></div>
        </div>
      </div>
      <div class="trakt-watchlist-epg-list custom-scroll" id="traktWatchlistEpgList"></div>
    </div>
  `;

  const searchMount = document.getElementById("traktWlEpgSearchMount");
  if (searchMount) {
    wlEpgState.searchBoxCtrl = createLiveSearchBox({
      inputId: "traktWlEpgSearchInput",
      clearBtnId: "traktWlEpgClearSearchBtn",
      initialValue: wlEpgState.query,
      onFilter: (q) => {
        wlEpgState.query = q;
        renderWatchlistEpgList();
      },
    });
    searchMount.appendChild(wlEpgState.searchBoxCtrl.element);
  }

  const collapseMount = document.getElementById("traktWlEpgCollapseAllMount");
  if (collapseMount) {
    wlEpgState.collapseAllCtrl = createCollapseAllButton({
      onToggle: (allCollapsed) => {
        const listEl = document.getElementById("traktWatchlistEpgList");
        setAllGroupsCollapsed(
          listEl,
          wlEpgState.collapsedKeys,
          !allCollapsed,
        );
        syncWlEpgCollapseAllButton();
      },
    });
    collapseMount.appendChild(wlEpgState.collapseAllCtrl.element);
  }

  const sortMount = document.getElementById("traktWlEpgSortMount");
  if (sortMount) {
    wlEpgState.sortControlsCtrl = createMultiSortControls({
      initialLevels: wlEpgState.sortLevels,
      defaultKey: "title",
      onChange: (levels) => {
        wlEpgState.sortLevels = levels;
        const serialized = JSON.stringify(levels);
        if (state.settings) state.settings.watchlist_epg_sort_levels = serialized;
        invoke("set_setting", {
          key: "watchlist_epg_sort_levels",
          value: serialized,
        }).catch(() => {});
        renderWatchlistEpgList();
      },
    });
    sortMount.appendChild(wlEpgState.sortControlsCtrl.element);
  }

  const filterBtns = viewport.querySelectorAll("[data-wl-epg-filter]");
  filterBtns.forEach((btn) => {
    btn.onclick = () => {
      wlEpgState.filter = btn.dataset.wlEpgFilter;
      filterBtns.forEach((b) =>
        b.classList.toggle("active", b.dataset.wlEpgFilter === wlEpgState.filter),
      );
      renderWatchlistEpgList();
    };
  });

  const pastCheck = document.getElementById("traktWlEpgIncludePastCheck");
  if (pastCheck) {
    pastCheck.onchange = () => {
      wlEpgState.includePast = pastCheck.checked;
      fetchAndRenderWatchlistEpg();
    };
  }

  if (options.reuseAirings) {
    renderWatchlistEpgList();
  } else {
    await fetchAndRenderWatchlistEpg();
  }
}

export async function refreshWatchlistEpgIfActive() {
  const listEl = document.getElementById("traktWatchlistEpgList");
  if (!listEl || !listEl.isConnected) return;
  await fetchAndRenderWatchlistEpg({ silent: true, preserveScroll: true });
}
