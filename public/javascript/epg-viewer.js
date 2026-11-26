import { state, invoke, setFavoritesList } from "./application/state.js";
import {
  t,
  setLanguage,
  addLanguageChangeListener,
} from "./application/i18n.js";
import { Countries } from "./utils/countries.js";
import { Icons } from "./utils/icons.js";
import { registerTraktIcons } from "./trakt/trakt-icons.js";
import {
  updateAllFavBtnState,
  updateLayoutButtonUi,
  updateGridCornerButton,
  renderEmptySourcesState,
} from "./application/ui.js";
import {
  initSources,
  refreshSourcesFromDb,
  loadSourcesDropdown,
} from "./application/sources.js";
import {
  initFavorites,
  renderFavoritesList,
} from "./application/favorites.js";
import { initChannels } from "./application/channels.js";
import {
  initEpg,
  fetchEPG,
  navigateDate,
  setupDateSelector,
  renderEPG,
} from "./application/epg.js";
import { initTmdb } from "./tmdb/tmdb.js";
import { initModal } from "./application/modal.js";
import { initSettings } from "./application/settings.js";
import { scheduleStartupUpdateCheck } from "./application/updater.js";
import { initTraktUI } from "./trakt/trakt-ui.js";
import { initGoogleUI } from "./google/google-ui.js";

function initHeaderScroll() {
  const scrollArea = document.getElementById("headerScrollArea");
  const leftBtn = document.getElementById("headerScrollLeftBtn");
  const rightBtn = document.getElementById("headerScrollRightBtn");
  if (!scrollArea || !leftBtn || !rightBtn) return;

  const updateScrollButtons = () => {
    const maxScroll = scrollArea.scrollWidth - scrollArea.clientWidth;
    const hasOverflow = maxScroll > 2;
    leftBtn.classList.toggle("u-hidden", !hasOverflow || scrollArea.scrollLeft <= 1);
    rightBtn.classList.toggle("u-hidden", !hasOverflow || scrollArea.scrollLeft >= maxScroll - 1);
  };

  leftBtn.onclick = () => {
    scrollArea.scrollBy({ left: -220, behavior: "smooth" });
  };
  rightBtn.onclick = () => {
    scrollArea.scrollBy({ left: 220, behavior: "smooth" });
  };

  scrollArea.addEventListener(
    "wheel",
    (e) => {
      if (scrollArea.scrollWidth <= scrollArea.clientWidth) return;
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        e.preventDefault();
        scrollArea.scrollLeft += e.deltaY;
      }
    },
    { passive: false },
  );

  scrollArea.addEventListener("scroll", updateScrollButtons, { passive: true });
  window.addEventListener("resize", updateScrollButtons);

  if (typeof ResizeObserver === "function") {
    const ro = new ResizeObserver(updateScrollButtons);
    ro.observe(scrollArea);
  }
  if (typeof MutationObserver === "function") {
    const mo = new MutationObserver(() => requestAnimationFrame(updateScrollButtons));
    mo.observe(scrollArea, {
      subtree: true,
      attributes: true,
      attributeFilter: ["class"],
      childList: true,
    });
  }

  requestAnimationFrame(updateScrollButtons);
}

async function init() {
  registerTraktIcons();
  Countries.populateSelect("sourceCountrySelect", {
    format: "source",
    selected: "HU",
  });
  Icons.replace();

  initHeaderScroll();
  initSources();
  initChannels();
  initEpg();
  initTmdb();
  initModal();
  initSettings();

  try {
    state.settings = await invoke("get_settings");
  } catch (e) {
    console.error("Nem sikerült lekérni a beállításokat:", e);
    state.settings = {};
  }

  initFavorites();

  const initialLang = state.settings.language || "hungarian";
  await setLanguage(initialLang, {
    persist: !state.settings.language,
    notify: false,
  });

  addLanguageChangeListener(() => {
    updateLayoutButtonUi();
    updateGridCornerButton();
    setupDateSelector();
    renderEPG();
    loadSourcesDropdown();
    if (
      document
        .getElementById("favoritesModalOverlay")
        ?.classList.contains("active")
    ) {
      renderFavoritesList();
    }
  });

  await initTraktUI();
  await initGoogleUI();

  if (state.settings.view_step_min) {
    state.currentStepMin = parseInt(state.settings.view_step_min, 10);
  }
  if (state.settings.view_minute_height) {
    state.currentMinuteHeight = parseFloat(state.settings.view_minute_height);
  }
  if (
    state.settings.view_layout === "horizontal" ||
    state.settings.view_layout === "vertical"
  ) {
    state.currentLayout = state.settings.view_layout;
  }

  updateLayoutButtonUi();

  const viewSelect = document.getElementById("viewSelect");
  if (viewSelect) {
    viewSelect.value = String(state.currentStepMin);
  }

  const prevDayBtn = document.getElementById("prevDayBtn");
  if (prevDayBtn) prevDayBtn.onclick = () => navigateDate(-1);
  const nextDayBtn = document.getElementById("nextDayBtn");
  if (nextDayBtn) nextDayBtn.onclick = () => navigateDate(1);

  try {
    await refreshSourcesFromDb();
  } catch (err) {
    console.error("Failed to load sources during init:", err);
    state.sources = [];
  }
  try {
    setFavoritesList(await invoke("get_favorites"));
  } catch (err) {
    console.error("Failed to load favorites during init:", err);
    setFavoritesList([]);
  }

  if (state.sources.length > 0) {
    if (
      state.settings.current_source_id &&
      (state.settings.current_source_id === "__ALL_FAVORITES__" ||
        state.sources.some((s) => s.id === state.settings.current_source_id))
    ) {
      state.currentSourceId = state.settings.current_source_id;
    } else {
      state.currentSourceId = "__ALL_FAVORITES__";
    }
    document.getElementById("sourceSelect").value = state.currentSourceId;
    updateAllFavBtnState();
    await fetchEPG();
  } else {
    state.currentSourceId = "";
    updateAllFavBtnState();
    document.getElementById("status").textContent = t("status_ready");
    renderEmptySourcesState();
  }

  scheduleStartupUpdateCheck();
}

if (document.readyState === "loading") {
  window.addEventListener("DOMContentLoaded", init, { once: true });
} else {
  init();
}
