import { state, invoke } from "./state.js";
import { t, getCurrentLang } from "./i18n.js";
import { Icons } from "../utils/icons.js";
import { Countries } from "../utils/countries.js";
import { SearchUtils } from "../utils/search.js";

export function iconHtml(name, size = 16) {
  return Icons.get(name, { size });
}

// ============================================================================
// Fuzzy Search & Diacritics Normalization Utilities (delegated to SearchUtils)
// ============================================================================

export function normalizeSearchText(str) {
  return SearchUtils.normalizeText(str);
}

export function levenshteinDistance(a, b) {
  return SearchUtils.damerauLevenshtein(a, b);
}

export function fuzzySearchMatch(target, query, options) {
  return SearchUtils.fuzzyMatch(target, query, options);
}

export function showDialog({ title, message, isConfirm = false }) {
  return new Promise((resolve) => {
    const titleEl = document.getElementById("customConfirmTitle");
    const msgEl = document.getElementById("customConfirmMessage");
    const overlay = document.getElementById("customConfirmOverlay");
    const okBtn = document.getElementById("customConfirmOkBtn");
    const cancelBtn = document.getElementById("customConfirmCancelBtn");
    const closeBtn = document.getElementById("customConfirmClose");

    if (titleEl) titleEl.textContent = title;
    if (msgEl) msgEl.textContent = message;

    if (isConfirm) {
      if (cancelBtn) {
        cancelBtn.classList.remove("u-hidden");
        cancelBtn.textContent = t("btn_cancel");
      }
    } else {
      if (cancelBtn) cancelBtn.classList.add("u-hidden");
    }
    if (okBtn) okBtn.textContent = t("btn_ok");

    const cleanup = (result) => {
      if (overlay) overlay.classList.remove("active");
      if (okBtn) okBtn.onclick = null;
      if (cancelBtn) cancelBtn.onclick = null;
      if (closeBtn) closeBtn.onclick = null;
      resolve(result);
    };

    if (okBtn) okBtn.onclick = () => cleanup(true);
    if (cancelBtn) cancelBtn.onclick = () => cleanup(false);
    if (closeBtn) closeBtn.onclick = () => cleanup(false);

    if (overlay) overlay.classList.add("active");
  });
}

export function showConfirm(title, message) {
  return showDialog({ title, message, isConfirm: true });
}

export function showAlert(title, message) {
  return showDialog({ title, message, isConfirm: false }).then(() => {});
}

export function showPrompt(title, message, defaultValue = "") {
  return new Promise((resolve) => {
    document.getElementById("customPromptTitle").textContent = title;
    document.getElementById("customPromptMessage").textContent = message;
    const overlay = document.getElementById("customPromptOverlay");
    const input = document.getElementById("customPromptInput");
    const okBtn = document.getElementById("customPromptOkBtn");
    const cancelBtn = document.getElementById("customPromptCancelBtn");
    const closeBtn = document.getElementById("customPromptClose");

    input.value = defaultValue;
    okBtn.textContent = t("btn_ok");
    cancelBtn.textContent = t("btn_cancel");

    const cleanup = (val) => {
      overlay.classList.remove("active");
      okBtn.onclick = null;
      cancelBtn.onclick = null;
      closeBtn.onclick = null;
      input.onkeydown = null;
      resolve(val);
    };

    okBtn.onclick = () => cleanup(input.value.trim());
    cancelBtn.onclick = () => cleanup(null);
    closeBtn.onclick = () => cleanup(null);
    input.onkeydown = (e) => {
      if (e.key === "Enter") cleanup(input.value.trim());
      if (e.key === "Escape") cleanup(null);
    };

    overlay.classList.add("active");
    setTimeout(() => {
      input.focus();
      input.select();
    }, 60);
  });
}

export async function openLogoSearchModal(channelName, countryCode = "HU") {
  const modal = document.getElementById("logoSearchModal");
  const closeBtn = document.getElementById("logoSearchClose");
  const input = document.getElementById("logoSearchInput");
  const countrySelect = document.getElementById("logoSearchCountrySelect");
  const execBtn = document.getElementById("logoSearchExecBtn");
  const statusEl = document.getElementById("logoSearchStatus");
  const gridEl = document.getElementById("logoSearchResultsGrid");

  modal.classList.remove("u-hidden");
  modal.classList.add("active");

  let initialQuery = channelName || "";
  try {
    const [cleaned] = await invoke("clean_and_slugify_channel", { name: initialQuery });
    if (cleaned) initialQuery = cleaned;
  } catch (_) {
    // fallback: use raw name
  }

  input.value = initialQuery;
  Countries.populateSelect(countrySelect, {
    format: "search",
    selected: countryCode || "HU",
  });

  return new Promise((resolve) => {
    const doSearch = async () => {
      const q = input.value.trim();
      const c = countrySelect.value;
      if (!q) return;

      statusEl.classList.remove("u-hidden");
      statusEl.textContent = t("logo_search_searching");
      gridEl.innerHTML = "";
      execBtn.disabled = true;

      try {
        const results = await invoke("search_channel_logos", {
          query: q,
          countryCode: c,
        });

        statusEl.classList.add("u-hidden");
        execBtn.disabled = false;

        if (!results || results.length === 0) {
          statusEl.classList.remove("u-hidden");
          statusEl.textContent = t("logo_search_no_results");
          return;
        }

        results.forEach((item) => {
          const card = document.createElement("div");
          card.className = "logo-card";

          let badgeClass = "badge-tv-logo";
          if (item.sourceName === "iptv-org") badgeClass = "badge-iptv-org";
          else if (item.sourceName.includes("Wikimedia"))
            badgeClass = "badge-wikimedia";

          card.innerHTML = `
            <div class="logo-card-img-wrap"></div>
            <div class="logo-card-title" title="${escapeHtml(item.title)}">${escapeHtml(item.title)}</div>
            <div class="logo-card-meta">
              <span class="logo-source-badge ${badgeClass}">${escapeHtml(item.sourceName)}</span>
              <span class="logo-format-badge">${escapeHtml(item.format)}</span>
            </div>
          `;

          const imgWrap = card.querySelector(".logo-card-img-wrap");
          appendSafeImage(imgWrap, item.thumbUrl || item.url, {
            onErrorMode: "errSpan",
          });

          card.onclick = () => {
            closeModal(item.url);
          };

          gridEl.appendChild(card);
        });
      } catch (err) {
        statusEl.classList.remove("u-hidden");
        statusEl.textContent = t("logo_search_error") + " " + err;
        execBtn.disabled = false;
      }
    };

    const onBackdropClick = (e) => {
      if (e.target === modal) {
        e.stopPropagation();
        closeModal(null);
      }
    };
    modal.addEventListener("click", onBackdropClick);

    const closeModal = (url) => {
      modal.removeEventListener("click", onBackdropClick);
      modal.classList.add("u-hidden");
      modal.classList.remove("active");
      closeBtn.onclick = null;
      execBtn.onclick = null;
      input.onkeydown = null;
      resolve(url);
    };

    closeBtn.onclick = () => closeModal(null);
    execBtn.onclick = doSearch;
    input.onkeydown = (e) => {
      if (e.key === "Enter") doSearch();
    };

    doSearch();
  });
}

export function openChannelEditModal(channel) {
  return new Promise((resolve) => {
    const overlay = document.getElementById("channelEditModalOverlay");
    const origTitleEl = document.getElementById("channelEditOrigTitle");
    const nameInput = document.getElementById("channelEditNameInput");
    const logoInput = document.getElementById("channelEditLogoInput");
    const previewEl = document.getElementById("channelEditLogoPreview");
    const closeBtn = document.getElementById("channelEditCloseBtn");
    const cancelBtn = document.getElementById("channelEditCancelBtn");
    const saveBtn = document.getElementById("channelEditSaveBtn");
    const resetBtn = document.getElementById("channelEditResetBtn");
    const searchBtn = document.getElementById("channelEditLogoSearchBtn");

    const origName = channel.originalName || channel.name;
    origTitleEl.textContent = `${t("original_channel_name")}: ${origName} (ID: ${channel.id})`;

    nameInput.value = channel.customName || "";
    logoInput.value = channel.customIcon || "";

    let previewSeq = 0;
    const updatePreview = () => {
      const seq = ++previewSeq;
      const url = logoInput.value.trim();
      const currentEffectiveRaw = (channel.customIcon || channel.icon || "").trim();
      const useCached =
        channel.cachedIcon &&
        ((!url && !channel.customIcon) || (url && url === currentEffectiveRaw));
      const displayUrl = useCached
        ? channel.cachedIcon
        : url || channel.cachedIcon || channel.icon || "";
      previewEl.innerHTML = "";
      if (displayUrl) {
        const img = appendSafeImage(previewEl, displayUrl, {
          onErrorMode: "errSpan",
        });
        if (
          img &&
          !displayUrl.startsWith("http://epgimg.localhost/") &&
          (displayUrl.startsWith("http://") || displayUrl.startsWith("https://"))
        ) {
          img.onerror = () => {
            img.onerror = null;
            invoke("cache_logo", { url: displayUrl })
              .then((dataUri) => {
                if (seq !== previewSeq || !previewEl.isConnected) return;
                if (dataUri) {
                  previewEl.innerHTML = "";
                  appendSafeImage(previewEl, dataUri, { onErrorMode: "errSpan" });
                } else {
                  const span = document.createElement("span");
                  span.className = "img-error-fallback";
                  span.textContent = "ERR";
                  previewEl.replaceChildren(span);
                }
              })
              .catch(() => {
                if (seq !== previewSeq || !previewEl.isConnected) return;
                const span = document.createElement("span");
                span.className = "img-error-fallback";
                span.textContent = "ERR";
                previewEl.replaceChildren(span);
              });
          };
        }
      } else {
        const tvSvg = Icons.get("device-tv", { size: 24 }) || "TV";
        previewEl.innerHTML = `<span class="logo-preview-placeholder">${tvSvg}</span>`;
      }
    };

    updatePreview();
    logoInput.oninput = updatePreview;

    if (searchBtn) {
      searchBtn.onclick = async (e) => {
        if (e) e.stopPropagation();
        const queryName =
          nameInput.value.trim() ||
          channel.customName ||
          channel.originalName ||
          channel.name;
        const actualSourceId = channel.sourceId || state.currentSourceId;
        const srcObj = state.sources.find((s) => s.id === actualSourceId);
        const country =
          channel.countryCode || (srcObj ? srcObj.countryCode : null) || "HU";
        const selectedUrl = await openLogoSearchModal(queryName, country);
        if (selectedUrl) {
          logoInput.value = selectedUrl;
          updatePreview();
        }
      };
    }

    const cleanup = (result) => {
      overlay.classList.remove("active");
      closeBtn.onclick = null;
      cancelBtn.onclick = null;
      saveBtn.onclick = null;
      resetBtn.onclick = null;
      if (searchBtn) searchBtn.onclick = null;
      logoInput.oninput = null;
      resolve(result);
    };

    closeBtn.onclick = () => cleanup(null);
    cancelBtn.onclick = () => cleanup(null);

    saveBtn.onclick = () => {
      cleanup({
        action: "save",
        name: nameInput.value.trim(),
        logo: logoInput.value.trim(),
      });
    };

    resetBtn.onclick = () => {
      cleanup({
        action: "reset",
        name: "",
      });
    };

    overlay.classList.add("active");
    setTimeout(() => {
      nameInput.focus();
      nameInput.select();
    }, 60);
  });
}

export function updateGridCornerButton() {
  const btn = document.getElementById("gridHomeBtn");
  if (!btn) return;

  const iconEl = btn.querySelector(".grid-home-icon");
  const labelEl = btn.querySelector(".grid-home-label");

  if (state.activeSingleChannelId) {
    btn.classList.add("back-mode");
    btn.title = t("grid_back_btn_title");
    if (iconEl) {
      iconEl.setAttribute("data-icon", "arrow-left");
      iconEl.innerHTML = iconHtml("arrow-left", 18);
    }
    if (labelEl) {
      if (state.currentLayout === "horizontal") {
        labelEl.textContent = t("grid_back_btn_label");
        labelEl.classList.remove("u-hidden");
      } else {
        labelEl.textContent = "";
        labelEl.classList.add("u-hidden");
      }
    }
  } else {
    btn.classList.remove("back-mode");
    btn.title = t("grid_home_btn_title");
    if (iconEl) {
      iconEl.setAttribute("data-icon", "home");
      iconEl.innerHTML = iconHtml("home", 18);
    }
    if (labelEl) {
      if (state.currentLayout === "horizontal") {
        labelEl.textContent = t("grid_home_btn_label");
        labelEl.classList.remove("u-hidden");
      } else {
        labelEl.textContent = "";
        labelEl.classList.add("u-hidden");
      }
    }
  }
}

export function formatDayHeader(dateStr) {
  const date = new Date(dateStr + "T00:00:00");
  const months = t("months");
  const days = t("days");
  const y = date.getFullYear();
  const m = months[date.getMonth()];
  const d = date.getDate();
  const dayName = days[date.getDay()];

  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const target = new Date(date);
  target.setHours(0, 0, 0, 0);
  const isToday = Math.round((target - today) / (1000 * 60 * 60 * 24)) === 0;

  const fullDateStr =
    getCurrentLang() === "english" ? `${m} ${d}, ${y}` : `${y}. ${m} ${d}.`;

  let dayLabel = dayName;
  if (isToday) {
    dayLabel = `${dayName} (${t("day_today")})`;
  }

  return { fullDateStr, dayLabel, isToday };
}

export function updateAllFavBtnState() {
  const btn = document.getElementById("allFavChannelsBtn");
  if (btn) {
    const iconEl = document.getElementById("allFavChannelsIcon");
    if (state.currentSourceId === "__ALL_FAVORITES__") {
      btn.classList.add("active");
      if (iconEl) {
        iconEl.setAttribute("data-icon", "star-filled");
        iconEl.innerHTML = iconHtml("star-filled", 15);
      }
    } else {
      btn.classList.remove("active");
      if (iconEl) {
        iconEl.setAttribute("data-icon", "star");
        iconEl.innerHTML = iconHtml("star", 15);
      }
    }
  }
}

export function updateLayoutButtonUi() {
  const btn = document.getElementById("layoutToggleBtn");
  const icon = document.getElementById("layoutToggleIcon");
  const label = document.getElementById("layoutToggleLabel");
  if (!btn) return;

  if (state.currentLayout === "horizontal") {
    btn.classList.add("horizontal-active");
    if (icon) {
      icon.setAttribute("data-icon", "layout-columns");
      icon.innerHTML = iconHtml("layout-columns", 16);
    }
    if (label) {
      label.textContent = t("btn_layout_vertical");
    }
    btn.title = t("title_layout_toggle_vert");
  } else {
    btn.classList.remove("horizontal-active");
    if (icon) {
      icon.setAttribute("data-icon", "layout-rows");
      icon.innerHTML = iconHtml("layout-rows", 16);
    }
    if (label) {
      label.textContent = t("btn_layout_horizontal");
    }
    btn.title = t("title_layout_toggle");
  }
  updateGridCornerButton();
}

export function renderEmptySourcesState() {
  const activeBadge = document.getElementById("activeChannelBadge");
  if (activeBadge) activeBadge.classList.add("u-hidden");
  const container = document.getElementById("channelsContainer");
  container.innerHTML = `
                <div class="empty-grid-notice">
                    <h2>${t("empty_sources_title")}</h2>
                    <p>${t("empty_sources_desc", { btn: t("settings_btn") })}</p>
                    <button id="emptySourcesOpenSettingsBtn" class="btn btn-green">${t("empty_sources_btn")}</button>
                </div>
            `;
  const openBtn = document.getElementById("emptySourcesOpenSettingsBtn");
  if (openBtn) {
    openBtn.onclick = () => document.getElementById("settingsBtn")?.click();
  }
  document.getElementById("timeColumn").innerHTML = "";
  document.getElementById("dateSelect").innerHTML = "";
}

export function renderEmptyFavoritesState() {
  const activeBadge = document.getElementById("activeChannelBadge");
  if (activeBadge) activeBadge.classList.add("u-hidden");
  document.getElementById("channelsContainer").innerHTML = `
                <div class="empty-grid-notice">
                    <h2>${t("no_favorite_channels_title")}</h2>
                    <p>${t("no_favorite_channels_desc")}</p>
                </div>
            `;
  document.getElementById("timeColumn").innerHTML = "";
  document.getElementById("dateSelect").innerHTML = "";
}

export function escapeHtml(text) {
  if (text === null || text === undefined) return "";
  return String(text)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

export function isSafeImageSrc(url) {
  if (!url || typeof url !== "string") return false;
  const trimmed = url.trim();
  if (!trimmed) return false;
  const lower = trimmed.toLowerCase();
  if (
    lower.startsWith("http://") ||
    lower.startsWith("https://") ||
    lower.startsWith("epgimg://") ||
    lower.startsWith("data:image/") ||
    lower.startsWith("blob:")
  ) {
    return true;
  }
  // Allow relative paths without a URI scheme
  return !/^[a-z][a-z0-9+.-]*:/i.test(trimmed);
}

export function appendSafeImage(container, src, options = {}) {
  if (!container || !isSafeImageSrc(src)) return null;
  const img = document.createElement("img");
  if (options.className) img.className = options.className;
  img.alt = "";
  img.src = src.trim();
  img.onerror = () => {
    img.onerror = null;
    if (options.onErrorMode === "errSpan") {
      const span = document.createElement("span");
      span.className = "img-error-fallback";
      span.textContent = "ERR";
      container.replaceChildren(span);
    } else if (options.onErrorMode === "errText") {
      container.textContent = "ERR";
    } else {
      img.classList.add("u-hidden");
    }
  };
  if (options.prepend) {
    container.prepend(img);
  } else {
    container.appendChild(img);
  }
  return img;
}

export function formatByteSize(bytes) {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / (1024 * 1024)).toFixed(2) + " MB";
}

export function setExportImportStatus(elementId, text, type = "loading") {
  const el = document.getElementById(elementId);
  if (!el) return;
  el.className = `export-import-status status-${type}`;
  el.textContent = text;
  el.classList.remove("u-hidden");
  if (type !== "loading") {
    setTimeout(() => {
      if (el.textContent === text) {
        el.classList.add("u-hidden");
      }
    }, 6000);
  }
}
