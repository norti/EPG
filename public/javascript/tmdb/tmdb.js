import { state, invoke } from "../application/state.js";
import { t, getCurrentLang } from "../application/i18n.js";
import {
  iconHtml,
  showConfirm,
  showAlert,
  escapeHtml,
} from "../application/ui.js";
import { attachToModal, detachFromModal } from "../trakt/trakt-ui.js";

export function getTmdbLang(countryCode) {
  const code = (countryCode || "").toUpperCase();
  const map = {
    HU: "hu-HU",
    DE: "de-DE",
    EN: "en-US",
    US: "en-US",
    UK: "en-GB",
    FR: "fr-FR",
    ES: "es-ES",
    IT: "it-IT",
    SK: "sk-SK",
    RO: "ro-RO",
    PL: "pl-PL",
    PT: "pt-PT",
    CZ: "cs-CZ",
  };
  return map[code] || (getCurrentLang() === "english" ? "en-US" : "hu-HU");
}

export function copyText(text, badgeEl) {
  if (!text) return;
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard
      .writeText(text)
      .then(() => {
        if (badgeEl) {
          const origHtml = badgeEl.innerHTML;
          badgeEl.innerHTML = `${iconHtml("check", 13)} ${escapeHtml(t("tmdb_copied_to_clipboard"))}`;
          setTimeout(() => {
            badgeEl.innerHTML = origHtml;
          }, 1400);
        }
      })
      .catch(() => {});
  }
}

let currentModalLangSwitchIndex = 0;
let currentModalPosterPath = "";
let currentModalLargePosterSrc = "";
let fixSearchDebounce = null;
let fixSearchSeq = 0;
let posterZoomDebounce = null;

function getLargePosterUrl(posterPath) {
  if (!posterPath) return "";
  return `https://image.tmdb.org/t/p/original${posterPath}`;
}

async function loadCachedTmdbImage(url, isStillCurrent, applySrc) {
  if (!url) return;
  try {
    const cached = await invoke("get_cached_logo", { url });
    if (!isStillCurrent()) return;
    if (cached) {
      applySrc(cached);
      return;
    }
    const downloaded = await invoke("cache_logo", { url });
    if (!isStillCurrent()) return;
    if (downloaded) {
      applySrc(downloaded);
      return;
    }
  } catch (_) {}
  if (isStillCurrent()) {
    applySrc(url);
  }
}

async function preloadAndCacheLargePoster(posterPath) {
  if (!posterPath) return;
  const largeUrl = getLargePosterUrl(posterPath);
  try {
    const cached = await invoke("get_cached_logo", { url: largeUrl });
    if (cached) {
      if (currentModalPosterPath === posterPath) {
        currentModalLargePosterSrc = cached;
      }
    } else {
      invoke("cache_logo", { url: largeUrl })
        .then((data) => {
          if (data && currentModalPosterPath === posterPath) {
            currentModalLargePosterSrc = data;
          }
        })
        .catch(() => {});
    }
  } catch (_) {}
}

function showPosterZoom(e) {
  if (!currentModalPosterPath) return;
  const popup = document.getElementById("posterZoomPopup");
  const img = document.getElementById("posterZoomImg");
  const loader = document.getElementById("posterZoomLoader");
  const modalPoster = document.getElementById("modalPoster");
  const largeUrl = getLargePosterUrl(currentModalPosterPath);

  popup.classList.remove("u-hidden");
  positionPosterZoom(e);

  // Azonnali helyes előnézet: ha a nagy méret már a memóriában van, azt használjuk,
  // egyébként a modálban már sikeresen betöltött w500-as posztert mutatjuk meg azonnal,
  // így véletlenül sem jelenhet meg egy korábbi műsor képe!
  const previewSrc =
    currentModalLargePosterSrc ||
    (modalPoster && modalPoster.src ? modalPoster.src : "") ||
    `https://image.tmdb.org/t/p/w500${currentModalPosterPath}`;

  if (img.dataset.forPosterPath !== currentModalPosterPath) {
    img.dataset.forPosterPath = currentModalPosterPath;
    img.src = previewSrc;
    loader.classList.toggle("u-hidden", !!currentModalLargePosterSrc);
  }

  // Ha a maximális méretű poszter még nincs a memóriában, lekérjük és zökkenőmentesen frissítjük
  if (!currentModalLargePosterSrc) {
    invoke("get_cached_logo", { url: largeUrl })
      .then((cached) => {
        if (cached) {
          if (img.dataset.forPosterPath === currentModalPosterPath) {
            currentModalLargePosterSrc = cached;
            if (img.src !== cached) {
              img.src = cached;
            }
            loader.classList.add("u-hidden");
          }
        } else {
          invoke("cache_logo", { url: largeUrl })
            .then((data) => {
              if (data && img.dataset.forPosterPath === currentModalPosterPath) {
                currentModalLargePosterSrc = data;
                if (img.src !== data) {
                  img.src = data;
                }
                loader.classList.add("u-hidden");
              }
            })
            .catch(() => {
              loader.classList.add("u-hidden");
            });
        }
      })
      .catch(() => {
        loader.classList.add("u-hidden");
      });
  } else {
    loader.classList.add("u-hidden");
  }

  requestAnimationFrame(() => {
    popup.classList.add("visible");
  });
}

export function hidePosterZoom() {
  if (posterZoomDebounce) clearTimeout(posterZoomDebounce);
  const popup = document.getElementById("posterZoomPopup");
  const img = document.getElementById("posterZoomImg");
  const loader = document.getElementById("posterZoomLoader");
  if (popup) {
    popup.classList.remove("visible");
    popup.classList.add("u-hidden");
  }
  if (img) {
    img.src = "";
    img.dataset.forPosterPath = "";
  }
  if (loader) {
    loader.classList.add("u-hidden");
  }
}

function positionPosterZoom(e) {
  const popup = document.getElementById("posterZoomPopup");
  if (!popup || popup.classList.contains("u-hidden")) return;

  const pad = 16;
  const popW = popup.offsetWidth || 380;
  const popH = popup.offsetHeight || 570;

  let left = e.clientX + 20;
  let top = e.clientY - 60;

  // If overflowing on the right, place to the left of the cursor
  if (left + popW > window.innerWidth - pad) {
    left = Math.max(pad, e.clientX - popW - 20);
  }

  // Keep inside vertical screen bounds
  if (top + popH > window.innerHeight - pad) {
    top = Math.max(pad, window.innerHeight - popH - pad);
  }
  if (top < pad) {
    top = pad;
  }

  popup.style.left = `${left}px`;
  popup.style.top = `${top}px`;
}

export function updateModalLeftColVisibility() {
  const leftCol = document.getElementById("modalLeftCol");
  const posterCol = document.getElementById("modalPosterCol");
  const posterTraktMeta = document.getElementById("modalPosterTraktMeta");
  if (!leftCol) return;
  const hasPoster = posterCol && !posterCol.classList.contains("u-hidden");
  const hasTraktMeta =
    posterTraktMeta && !posterTraktMeta.classList.contains("u-hidden");
  leftCol.classList.toggle("u-hidden", !(hasPoster || hasTraktMeta));
}

export function resetModalTmdbUi(preserveTrakt = false) {
  hidePosterZoom();
  currentModalLangSwitchIndex++;
  currentModalLargePosterSrc = "";
  const posterCol = document.getElementById("modalPosterCol");
  const posterImg = document.getElementById("modalPoster");
  const backdrop = document.getElementById("modalBackdrop");
  const mediaBadge = document.getElementById("modalMediaBadge");
  const ratingBadge = document.getElementById("modalRatingBadge");
  const yearBadge = document.getElementById("modalYearBadge");
  const variantsRow = document.getElementById("modalTitleVariants");
  const huBadge = document.getElementById("modalHuTitleBadge");
  const enBadge = document.getElementById("modalEnTitleBadge");
  const origBadge = document.getElementById("modalOrigTitleBadge");
  const tmdbLoading = document.getElementById("modalTmdbLoading");
  const tmdbOverview = document.getElementById("modalTmdbOverview");
  const tmdbOverviewText = document.getElementById("modalTmdbOverviewText");

  posterCol.classList.add("u-hidden");
  posterImg.src = "";
  backdrop.classList.add("u-hidden");
  backdrop.style.backgroundImage = "";
  mediaBadge.classList.add("u-hidden");
  ratingBadge.classList.add("u-hidden");
  yearBadge.classList.add("u-hidden");
  variantsRow.classList.add("u-hidden");
  huBadge.classList.add("u-hidden");
  huBadge.classList.remove("active");
  enBadge.classList.add("u-hidden");
  enBadge.classList.remove("active");
  origBadge.classList.add("u-hidden");
  origBadge.classList.remove("active");
  tmdbOverview.classList.add("u-hidden");
  tmdbOverviewText.textContent = "";
  tmdbLoading.classList.add("u-hidden");
  if (!preserveTrakt) {
    detachFromModal();
  }
  updateModalLeftColVisibility();
}

export function getModalTitleVariants(tmdbData, appLang) {
  if (!tmdbData) return [];

  const lang = (appLang || getCurrentLang() || "hungarian").toLowerCase();
  const isEnglishApp = lang === "english" || lang === "en";

  const huTitle = (
    tmdbData.huTitle ||
    tmdbData.enTitle ||
    tmdbData.originalTitle ||
    tmdbData.title ||
    ""
  ).trim();
  const enTitle = (
    tmdbData.enTitle ||
    tmdbData.originalTitle ||
    tmdbData.title ||
    ""
  ).trim();
  const origTitle = (tmdbData.originalTitle || "").trim();

  const variants = [];

  // 1. App-configured language first (when not English, English follows in 2nd place)
  if (!isEnglishApp && huTitle) {
    variants.push({
      slot: "hu",
      tmdbLang: "hu-HU",
      title: huTitle,
    });
  }

  // 2. English title in all cases (1st when the app itself is set to English)
  if (enTitle) {
    variants.push({
      slot: "en",
      tmdbLang: "en-US",
      title: enTitle,
    });
  }

  // 3. Original title (if present), even when identical to the other titles
  if (origTitle) {
    const origLang = tmdbData.originalLanguage
      ? langCodeToTmdbLang(tmdbData.originalLanguage)
      : "en-US";
    variants.push({
      slot: "orig",
      tmdbLang: origLang,
      title: origTitle,
    });
  }

  return variants;
}

function setActiveTitleVariantBadge(activeBadge) {
  ["modalHuTitleBadge", "modalEnTitleBadge", "modalOrigTitleBadge"].forEach(
    (id) => {
      const el = document.getElementById(id);
      if (!el) return;
      if (el === activeBadge) el.classList.add("active");
      else el.classList.remove("active");
    },
  );
}

export function renderTmdbDetailsInModal(tmdbData, options = {}) {
  resetModalTmdbUi(!!options.preserveTrakt);
  if (!tmdbData) return;

  const posterCol = document.getElementById("modalPosterCol");
  const posterImg = document.getElementById("modalPoster");
  const backdrop = document.getElementById("modalBackdrop");
  const mediaBadge = document.getElementById("modalMediaBadge");
  const ratingBadge = document.getElementById("modalRatingBadge");
  const yearBadge = document.getElementById("modalYearBadge");
  const variantsRow = document.getElementById("modalTitleVariants");
  const huBadge = document.getElementById("modalHuTitleBadge");
  const enBadge = document.getElementById("modalEnTitleBadge");
  const origBadge = document.getElementById("modalOrigTitleBadge");
  const tmdbOverview = document.getElementById("modalTmdbOverview");
  const tmdbOverviewText = document.getElementById("modalTmdbOverviewText");

  const renderFetchIndex = state.currentModalFetchIndex;
  const renderSwitchIndex = currentModalLangSwitchIndex;
  const isCurrentRender = () =>
    renderFetchIndex === state.currentModalFetchIndex &&
    renderSwitchIndex === currentModalLangSwitchIndex;

  // Poster (served through images.db cache for offline & zero repeated CDN hits)
  if (tmdbData.posterPath) {
    currentModalPosterPath = tmdbData.posterPath;
    const posterUrl = `https://image.tmdb.org/t/p/w500${tmdbData.posterPath}`;
    posterCol.classList.remove("u-hidden");
    updateModalLeftColVisibility();
    loadCachedTmdbImage(posterUrl, isCurrentRender, (src) => {
      posterImg.src = src;
    });
    preloadAndCacheLargePoster(tmdbData.posterPath);
  } else {
    currentModalPosterPath = "";
  }

  // Backdrop (served through images.db cache)
  if (tmdbData.backdropPath) {
    const backdropUrl = `https://image.tmdb.org/t/p/w780${tmdbData.backdropPath}`;
    backdrop.classList.remove("u-hidden");
    loadCachedTmdbImage(backdropUrl, isCurrentRender, (src) => {
      backdrop.style.backgroundImage = `url('${src}')`;
    });
  }

  // Media type badge
  if (tmdbData.mediaType) {
    mediaBadge.textContent =
      tmdbData.mediaType === "tv" ? t("media_type_tv") : t("media_type_movie");
    mediaBadge.classList.remove("u-hidden");
  }

  // Rating badge
  if (tmdbData.voteAverage && tmdbData.voteAverage > 0) {
    ratingBadge.innerHTML = `${iconHtml("star-filled", 12)} ${tmdbData.voteAverage.toFixed(1)} / 10`;
    ratingBadge.classList.remove("u-hidden");
  }

  // Release year badge
  if (tmdbData.releaseDate && tmdbData.releaseDate.length >= 4) {
    yearBadge.textContent = tmdbData.releaseDate.substring(0, 4);
    yearBadge.classList.remove("u-hidden");
  }

  // Title variants badges: 1. app language, 2. English, 3. Original (if present)
  const variants = getModalTitleVariants(tmdbData, getCurrentLang());
  const badgeMap = {
    hu: huBadge,
    en: enBadge,
    orig: origBadge,
  };
  const activeLang = (options.activeLang || getCurrentModalLang() || "").toLowerCase();
  const activeLangPrefix = activeLang.split("-")[0];
  let initialActiveBadge = null;

  variants.forEach((variant) => {
    const badgeEl = badgeMap[variant.slot];
    if (!badgeEl) return;

    if (variant.slot === "hu") {
      badgeEl.textContent = `🇭🇺 ${t("badge_hu")} ${variant.title}`;
    } else if (variant.slot === "en") {
      badgeEl.textContent = `🇬🇧 ${t("badge_en")} ${variant.title}`;
    } else if (variant.slot === "orig") {
      badgeEl.innerHTML = `${iconHtml("world", 13)} ${escapeHtml(t("badge_orig"))} ${escapeHtml(variant.title)}`;
    }

    badgeEl.classList.remove("u-hidden");
    badgeEl.onclick = () => switchTmdbLang(tmdbData, variant.tmdbLang, badgeEl);
    variantsRow.appendChild(badgeEl);

    if (!initialActiveBadge) {
      const variantPrefix = variant.tmdbLang.toLowerCase().split("-")[0];
      if (variantPrefix === activeLangPrefix) {
        initialActiveBadge = badgeEl;
      }
    }
  });

  if (variants.length > 0) {
    variantsRow.classList.remove("u-hidden");
    setActiveTitleVariantBadge(initialActiveBadge);
  }

  // TMDB Overview
  if (tmdbData.overview && tmdbData.overview.trim().length > 0) {
    tmdbOverviewText.textContent = tmdbData.overview;
    tmdbOverview.classList.remove("u-hidden");
  }
}

export function getCurrentModalLang() {
  const srcId = state.currentModalChannel?.sourceId || state.currentSourceId;
  const currentSrcObj = state.sources.find((s) => s.id === srcId);
  const countryCode =
    state.currentModalChannel?.countryCode ||
    (currentSrcObj ? currentSrcObj.countryCode : "HU");
  return getTmdbLang(countryCode);
}

function langCodeToTmdbLang(iso639) {
  const map = {
    hu: "hu-HU",
    en: "en-US",
    de: "de-DE",
    fr: "fr-FR",
    es: "es-ES",
    it: "it-IT",
    sk: "sk-SK",
    ro: "ro-RO",
    pl: "pl-PL",
    pt: "pt-PT",
    cs: "cs-CZ",
    ja: "ja-JP",
    ko: "ko-KR",
    zh: "zh-CN",
    ru: "ru-RU",
    ar: "ar-SA",
    tr: "tr-TR",
    nl: "nl-NL",
    sv: "sv-SE",
    da: "da-DK",
    no: "no-NO",
    fi: "fi-FI",
  };
  return (
    map[(iso639 || "").toLowerCase()] || `${iso639}-${iso639.toUpperCase()}`
  );
}

async function switchTmdbLang(tmdbData, targetLang, clickedBadgeEl) {
  if (!tmdbData || !tmdbData.tmdbId || !tmdbData.mediaType) return;
  const posterImg = document.getElementById("modalPoster");
  const posterCol = document.getElementById("modalPosterCol");
  const backdrop = document.getElementById("modalBackdrop");
  const tmdbOverview = document.getElementById("modalTmdbOverview");
  const tmdbOverviewText = document.getElementById("modalTmdbOverviewText");
  const tmdbLoading = document.getElementById("modalTmdbLoading");

  const fetchIndex = state.currentModalFetchIndex;
  const switchIndex = ++currentModalLangSwitchIndex;

  tmdbLoading.classList.remove("u-hidden");
  try {
    const details = await invoke("get_tmdb_details_for_lang", {
      tmdbId: tmdbData.tmdbId,
      mediaType: tmdbData.mediaType,
      lang: targetLang,
      queryTitle: state.currentModalTitle,
    });

    if (
      fetchIndex !== state.currentModalFetchIndex ||
      switchIndex !== currentModalLangSwitchIndex
    ) {
      return;
    }

    tmdbLoading.classList.add("u-hidden");

    if (clickedBadgeEl) {
      setActiveTitleVariantBadge(clickedBadgeEl);
    }

    const isCurrentSwitch = () =>
      fetchIndex === state.currentModalFetchIndex &&
      switchIndex === currentModalLangSwitchIndex;

    if (details.posterPath) {
      currentModalPosterPath = details.posterPath;
      currentModalLargePosterSrc = "";
      const posterUrl = `https://image.tmdb.org/t/p/w500${details.posterPath}`;
      posterCol.classList.remove("u-hidden");
      updateModalLeftColVisibility();
      loadCachedTmdbImage(posterUrl, isCurrentSwitch, (src) => {
        posterImg.src = src;
      });
      preloadAndCacheLargePoster(details.posterPath);
    }

    if (details.backdropPath) {
      const backdropUrl = `https://image.tmdb.org/t/p/w780${details.backdropPath}`;
      backdrop.classList.remove("u-hidden");
      loadCachedTmdbImage(backdropUrl, isCurrentSwitch, (src) => {
        backdrop.style.backgroundImage = `url('${src}')`;
      });
    }

    if (details.overview && details.overview.trim().length > 0) {
      tmdbOverviewText.textContent = details.overview;
      tmdbOverview.classList.remove("u-hidden");
    } else {
      tmdbOverview.classList.add("u-hidden");
      tmdbOverviewText.textContent = "";
    }
  } catch (e) {
    if (
      fetchIndex === state.currentModalFetchIndex &&
      switchIndex === currentModalLangSwitchIndex
    ) {
      tmdbLoading.classList.add("u-hidden");
    }
    console.warn("TMDB lang switch error:", e);
  }
}

export function openFixOverlay() {
  if (!state.currentModalTitle) return;
  const overlay = document.getElementById("modalFixOverlay");
  const programContext = document.getElementById("modalFixProgramContext");
  const programTitleEl = document.getElementById("modalFixProgramTitle");
  const programDescEl = document.getElementById("modalFixProgramDesc");
  const searchInput = document.getElementById("modalFixSearchInput");
  const clearBtn = document.getElementById("modalFixClearBtn");
  const statusEl = document.getElementById("modalFixStatus");
  const resultsEl = document.getElementById("modalFixResultsList");
  const modalContent = document.querySelector("#modalOverlay .modal-content");
  if (modalContent) modalContent.classList.add("has-fix-open");

  if (programContext && programTitleEl && programDescEl) {
    programTitleEl.textContent = state.currentModalTitle;
    const rawDesc = (state.currentModalDesc || "").trim();
    if (rawDesc) {
      programDescEl.textContent = rawDesc;
      programDescEl.classList.remove("is-empty");
    } else {
      programDescEl.textContent = t("modal_no_desc");
      programDescEl.classList.add("is-empty");
    }
    programContext.classList.remove("u-hidden");
  }

  overlay.classList.remove("u-hidden");
  searchInput.value = state.currentModalTitle;
  clearBtn.classList.toggle("u-hidden", !searchInput.value);
  statusEl.classList.add("u-hidden");
  resultsEl.innerHTML = "";

  setTimeout(() => {
    searchInput.focus();
    searchInput.select();
  }, 50);

  triggerFixSearch(state.currentModalTitle);
}

export function closeFixOverlay() {
  fixSearchSeq++;
  const overlay = document.getElementById("modalFixOverlay");
  if (overlay) overlay.classList.add("u-hidden");
  const modalContent = document.querySelector("#modalOverlay .modal-content");
  if (modalContent) modalContent.classList.remove("has-fix-open");
  if (fixSearchDebounce) {
    clearTimeout(fixSearchDebounce);
    fixSearchDebounce = null;
  }
}

async function triggerFixSearch(query) {
  const seq = ++fixSearchSeq;
  const statusEl = document.getElementById("modalFixStatus");
  const resultsEl = document.getElementById("modalFixResultsList");
  const spinner = document.getElementById("modalFixSpinner");

  const q = (query || "").trim();
  if (!q) {
    spinner.classList.add("u-hidden");
    resultsEl.innerHTML = "";
    statusEl.textContent = t("tmdb_fix_no_results");
    statusEl.classList.remove("u-hidden");
    return;
  }

  spinner.classList.remove("u-hidden");
  statusEl.textContent = t("tmdb_fix_searching");
  statusEl.classList.remove("u-hidden");
  resultsEl.innerHTML = "";

  const lang = getCurrentModalLang();

  try {
    const results = await invoke("search_tmdb", { query: q, lang });
    if (seq !== fixSearchSeq) return;
    spinner.classList.add("u-hidden");

    if (!results || results.length === 0) {
      statusEl.textContent = t("tmdb_fix_no_results");
      statusEl.classList.remove("u-hidden");
      return;
    }

    statusEl.classList.add("u-hidden");
    renderFixResults(results);
  } catch (e) {
    if (seq !== fixSearchSeq) return;
    spinner.classList.add("u-hidden");
    const errStr = String(e);
    if (errStr.includes("TMDB API")) {
      statusEl.textContent = t("tmdb_fix_api_key_missing");
    } else {
      statusEl.textContent = errStr;
    }
    statusEl.classList.remove("u-hidden");
  }
}

function renderFixResults(items) {
  const resultsEl = document.getElementById("modalFixResultsList");
  resultsEl.innerHTML = "";

  items.forEach((item) => {
    const card = document.createElement("div");
    card.className = "fix-result-card";

    // Poster
    if (item.posterPath) {
      const img = document.createElement("img");
      img.className = "fix-result-poster";
      img.src = `https://image.tmdb.org/t/p/w92${item.posterPath}`;
      img.alt = item.title || "Poster";
      card.appendChild(img);
    } else {
      const placeholder = document.createElement("div");
      placeholder.className = "fix-result-poster-placeholder";
      placeholder.innerHTML = iconHtml(item.mediaType === "tv" ? "device-tv" : "movie", 22);
      card.appendChild(placeholder);
    }

    // Info
    const info = document.createElement("div");
    info.className = "fix-result-info";

    const header = document.createElement("div");
    header.className = "fix-result-header";

    const title = document.createElement("span");
    title.className = "fix-result-title";
    title.textContent = item.title;
    header.appendChild(title);

    if (item.originalTitle && item.originalTitle !== item.title) {
      const orig = document.createElement("span");
      orig.className = "fix-result-orig-title";
      orig.textContent = `(${item.originalTitle})`;
      header.appendChild(orig);
    }

    const badges = document.createElement("div");
    badges.className = "fix-result-badges";

    const mediaBadge = document.createElement("span");
    mediaBadge.className = "tmdb-badge tmdb-badge-media";
    mediaBadge.textContent =
      item.mediaType === "tv" ? t("media_type_tv") : t("media_type_movie");
    badges.appendChild(mediaBadge);

    if (item.releaseDate && item.releaseDate.length >= 4) {
      const yearBadge = document.createElement("span");
      yearBadge.className = "tmdb-badge tmdb-badge-year";
      yearBadge.textContent = item.releaseDate.substring(0, 4);
      badges.appendChild(yearBadge);
    }

    if (item.voteAverage && item.voteAverage > 0) {
      const ratingBadge = document.createElement("span");
      ratingBadge.className = "tmdb-badge tmdb-badge-rating";
      ratingBadge.innerHTML = `${iconHtml("star-filled", 11)} ${item.voteAverage.toFixed(1)}`;
      badges.appendChild(ratingBadge);
    }

    header.appendChild(badges);
    info.appendChild(header);

    if (item.overview && item.overview.trim().length > 0) {
      const overview = document.createElement("div");
      overview.className = "fix-result-overview";
      overview.textContent = item.overview;
      info.appendChild(overview);
    }

    card.appendChild(info);

    card.onclick = async () => {
      if (card.classList.contains("selecting")) return;
      card.classList.add("selecting");
      const lang = getCurrentModalLang();
      try {
        const updatedDetails = await invoke("set_tmdb_match", {
          queryTitle: state.currentModalTitle,
          tmdbId: item.id,
          mediaType: item.mediaType,
          lang,
        });
        renderTmdbDetailsInModal(updatedDetails, { activeLang: lang });
        closeFixOverlay();
        attachToModal({
          title: state.currentModalTitle,
          tmdbId: updatedDetails?.tmdbId ?? null,
          mediaType: updatedDetails?.mediaType ?? null,
          tmdbData: updatedDetails,
        });
      } catch (err) {
        console.error("Hiba a TMDB egyezés mentésekor:", err);
        card.classList.remove("selecting");
        await showAlert(t("alert_error_title"), String(err));
      }
    };

    resultsEl.appendChild(card);
  });
}

async function removeTmdbMatch() {
  if (!state.currentModalTitle) return;
  const lang = getCurrentModalLang();
  try {
    await invoke("clear_tmdb_match", {
      queryTitle: state.currentModalTitle,
      lang,
    });
    resetModalTmdbUi();
    closeFixOverlay();
    attachToModal({
      title: state.currentModalTitle,
      tmdbId: null,
      mediaType: null,
      tmdbData: null,
    });
  } catch (e) {
    console.error("Hiba a TMDB egyezés törlésekor:", e);
  }
}

function setTmdbKeyStatusState(statusBadge, statusState) {
  if (!statusBadge) return;
  statusBadge.classList.remove(
    "tmdb-status--configured",
    "tmdb-status--unconfigured",
    "tmdb-status--valid",
    "tmdb-status--invalid",
  );
  if (statusState) {
    statusBadge.classList.add(`tmdb-status--${statusState}`);
  }
}

export async function loadTmdbSettings() {
  const statusBadge = document.getElementById("tmdbKeyStatusBadge");
  const keyInput = document.getElementById("tmdbApiKeyInput");
  const toggleBtn = document.getElementById("toggleTmdbKeyBtn");
  if (toggleBtn && keyInput) {
    toggleBtn.innerHTML = iconHtml(keyInput.type === "password" ? "eye" : "eye-off", 16);
  }
  try {
    const key = await invoke("get_tmdb_api_key");
    if (key && key.trim()) {
      keyInput.value = key;
      statusBadge.textContent = t("tmdb_status_configured");
      setTmdbKeyStatusState(statusBadge, "configured");
    } else {
      keyInput.value = "";
      statusBadge.textContent = t("tmdb_status_not_configured");
      setTmdbKeyStatusState(statusBadge, "unconfigured");
    }
  } catch (e) {
    console.error("Hiba a TMDB kulcs lekérésekor:", e);
  }
}

async function saveTmdbKey() {
  const key = document.getElementById("tmdbApiKeyInput").value.trim();
  const statusBadge = document.getElementById("tmdbKeyStatusBadge");
  try {
    await invoke("set_tmdb_api_key", { apiKey: key });
    if (key) {
      statusBadge.textContent = t("tmdb_status_configured");
      setTmdbKeyStatusState(statusBadge, "configured");
      await showAlert(t("alert_success_title"), t("tmdb_saved_success"));
    } else {
      statusBadge.textContent = t("tmdb_status_not_configured");
      setTmdbKeyStatusState(statusBadge, "unconfigured");
    }
  } catch (e) {
    await showAlert(t("alert_error_title"), String(e));
  }
}

async function clearTmdbKey() {
  document.getElementById("tmdbApiKeyInput").value = "";
  await saveTmdbKey();
}

async function testTmdbKey() {
  const key = document.getElementById("tmdbApiKeyInput").value.trim();
  const statusBadge = document.getElementById("tmdbKeyStatusBadge");
  if (!key) {
    statusBadge.textContent = t("tmdb_status_not_configured");
    setTmdbKeyStatusState(statusBadge, "unconfigured");
    return;
  }
  statusBadge.textContent = "...";
  try {
    const isValid = await invoke("test_tmdb_api_key", { apiKey: key });
    if (isValid) {
      statusBadge.textContent = t("tmdb_status_valid");
      setTmdbKeyStatusState(statusBadge, "valid");
    } else {
      statusBadge.textContent = t("tmdb_status_invalid");
      setTmdbKeyStatusState(statusBadge, "invalid");
    }
  } catch (e) {
    console.error("Hiba a TMDB kulcs tesztelésekor:", e);
    statusBadge.textContent = t("tmdb_status_invalid");
    setTmdbKeyStatusState(statusBadge, "invalid");
  }
}

async function clearTmdbCache() {
  const confirmed = await showConfirm(
    t("tmdb_btn_clear_cache"),
    t("confirm_clear_tmdb_cache"),
  );
  if (!confirmed) return;
  try {
    await invoke("clear_tmdb_cache");
    await showAlert(t("alert_success_title"), t("tmdb_cache_cleared"));
  } catch (e) {
    await showAlert(t("alert_error_title"), String(e));
  }
}

export function initTmdb() {
  document.getElementById("tmdbLogoLink").onclick = () => {
    invoke("open_url", { url: "https://www.themoviedb.org" });
  };

  // Modal poster hover enlarge listeners
  const modalPosterColEl = document.getElementById("modalPosterCol");
  modalPosterColEl.addEventListener("mouseenter", (e) => {
    if (posterZoomDebounce) clearTimeout(posterZoomDebounce);
    posterZoomDebounce = setTimeout(() => {
      showPosterZoom(e);
    }, 120);
  });
  modalPosterColEl.addEventListener("mousemove", (e) => {
    positionPosterZoom(e);
  });
  modalPosterColEl.addEventListener("mouseleave", () => {
    hidePosterZoom();
  });

  document.getElementById("modalFixBtn").onclick = () => {
    openFixOverlay();
  };

  document.getElementById("modalFixClose").onclick = () => {
    closeFixOverlay();
  };

  document.getElementById("modalFixSearchBtn").onclick = () => {
    if (fixSearchDebounce) clearTimeout(fixSearchDebounce);
    const input = document.getElementById("modalFixSearchInput");
    triggerFixSearch(input.value.trim());
  };

  document.getElementById("modalFixSearchInput").oninput = (e) => {
    const val = e.target.value;
    document
      .getElementById("modalFixClearBtn")
      .classList.toggle("u-hidden", !val);
    if (fixSearchDebounce) clearTimeout(fixSearchDebounce);
    fixSearchDebounce = setTimeout(() => {
      triggerFixSearch(val.trim());
    }, 350);
  };

  document.getElementById("modalFixSearchInput").onkeydown = (e) => {
    if (e.key === "Enter") {
      if (fixSearchDebounce) clearTimeout(fixSearchDebounce);
      triggerFixSearch(e.target.value.trim());
    } else if (e.key === "Escape") {
      closeFixOverlay();
    }
  };

  document.getElementById("modalFixClearBtn").onclick = () => {
    const input = document.getElementById("modalFixSearchInput");
    input.value = "";
    document.getElementById("modalFixClearBtn").classList.add("u-hidden");
    input.focus();
    triggerFixSearch("");
  };

  document.getElementById("modalFixRemoveBtn").onclick = () => {
    removeTmdbMatch();
  };

  document.getElementById("saveTmdbKeyBtn").onclick = saveTmdbKey;
  document.getElementById("clearTmdbKeyBtn").onclick = clearTmdbKey;
  document.getElementById("testTmdbKeyBtn").onclick = testTmdbKey;
  document.getElementById("clearTmdbCacheBtn").onclick = clearTmdbCache;
  document.getElementById("toggleTmdbKeyBtn").onclick = () => {
    const input = document.getElementById("tmdbApiKeyInput");
    const isHidden = input.type === "password";
    input.type = isHidden ? "text" : "password";
    const btn = document.getElementById("toggleTmdbKeyBtn");
    if (btn) {
      btn.innerHTML = iconHtml(isHidden ? "eye-off" : "eye", 16);
    }
  };
}
