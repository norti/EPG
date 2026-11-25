function getTmdbLang(countryCode) {
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
  return map[code] || (currentLang === "english" ? "en-US" : "hu-HU");
}

function copyText(text, badgeEl) {
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

let currentModalFetchIndex = 0;
let currentModalLangSwitchIndex = 0;
let currentModalChannel = null;
let currentModalPosterPath = "";
let currentModalLargePosterSrc = "";
let currentModalTmdbData = null;
let fixSearchDebounce = null;
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
    if (currentModalLargePosterSrc) {
      loader.style.display = "none";
    } else {
      loader.style.display = "flex";
    }
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
            loader.style.display = "none";
          }
        } else {
          invoke("cache_logo", { url: largeUrl })
            .then((data) => {
              if (data && img.dataset.forPosterPath === currentModalPosterPath) {
                currentModalLargePosterSrc = data;
                if (img.src !== data) {
                  img.src = data;
                }
                loader.style.display = "none";
              }
            })
            .catch(() => {
              loader.style.display = "none";
            });
        }
      })
      .catch(() => {
        loader.style.display = "none";
      });
  } else {
    loader.style.display = "none";
  }

  popup.style.display = "block";
  requestAnimationFrame(() => {
    popup.classList.add("visible");
  });
}

function hidePosterZoom() {
  if (posterZoomDebounce) clearTimeout(posterZoomDebounce);
  const popup = document.getElementById("posterZoomPopup");
  const img = document.getElementById("posterZoomImg");
  const loader = document.getElementById("posterZoomLoader");
  if (popup) {
    popup.classList.remove("visible");
    popup.style.display = "none";
  }
  if (img) {
    img.src = "";
    img.dataset.forPosterPath = "";
  }
  if (loader) {
    loader.style.display = "none";
  }
}

function positionPosterZoom(e) {
  const popup = document.getElementById("posterZoomPopup");
  if (!popup || popup.style.display === "none") return;

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

function updateModalLeftColVisibility() {
  const leftCol = document.getElementById("modalLeftCol");
  const posterCol = document.getElementById("modalPosterCol");
  const posterTraktMeta = document.getElementById("modalPosterTraktMeta");
  if (!leftCol) return;
  const hasPoster = posterCol && posterCol.style.display !== "none";
  const hasTraktMeta =
    posterTraktMeta && posterTraktMeta.style.display !== "none";
  leftCol.style.display = hasPoster || hasTraktMeta ? "flex" : "none";
}

function resetModalTmdbUi(preserveTrakt = false) {
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

  posterCol.style.display = "none";
  posterImg.src = "";
  backdrop.style.display = "none";
  backdrop.style.backgroundImage = "";
  mediaBadge.style.display = "none";
  ratingBadge.style.display = "none";
  yearBadge.style.display = "none";
  variantsRow.style.display = "none";
  huBadge.style.display = "none";
  huBadge.classList.remove("active");
  enBadge.style.display = "none";
  enBadge.classList.remove("active");
  origBadge.style.display = "none";
  origBadge.classList.remove("active");
  tmdbOverview.style.display = "none";
  tmdbOverviewText.textContent = "";
  tmdbLoading.style.display = "none";
  if (!preserveTrakt && window.TraktUI) {
    window.TraktUI.detachFromModal();
  }
  updateModalLeftColVisibility();
}

function getModalTitleVariants(tmdbData, appLang) {
  if (!tmdbData) return [];

  const lang = (
    appLang ||
    (typeof currentLang !== "undefined" ? currentLang : "hungarian")
  ).toLowerCase();
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

function renderTmdbDetailsInModal(tmdbData, options = {}) {
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

  const renderFetchIndex = currentModalFetchIndex;
  const renderSwitchIndex = currentModalLangSwitchIndex;
  const isCurrentRender = () =>
    renderFetchIndex === currentModalFetchIndex &&
    renderSwitchIndex === currentModalLangSwitchIndex;

  // Poster (served through images.db cache for offline & zero repeated CDN hits)
  if (tmdbData.posterPath) {
    currentModalPosterPath = tmdbData.posterPath;
    currentModalTmdbData = tmdbData;
    const posterUrl = `https://image.tmdb.org/t/p/w500${tmdbData.posterPath}`;
    posterCol.style.display = "block";
    updateModalLeftColVisibility();
    loadCachedTmdbImage(posterUrl, isCurrentRender, (src) => {
      posterImg.src = src;
    });
    preloadAndCacheLargePoster(tmdbData.posterPath);
  } else {
    currentModalPosterPath = "";
    currentModalTmdbData = tmdbData;
  }

  // Backdrop (served through images.db cache)
  if (tmdbData.backdropPath) {
    const backdropUrl = `https://image.tmdb.org/t/p/w780${tmdbData.backdropPath}`;
    backdrop.style.display = "block";
    loadCachedTmdbImage(backdropUrl, isCurrentRender, (src) => {
      backdrop.style.backgroundImage = `url('${src}')`;
    });
  }

  // Media type badge
  if (tmdbData.mediaType) {
    mediaBadge.textContent =
      tmdbData.mediaType === "tv" ? t("media_type_tv") : t("media_type_movie");
    mediaBadge.style.display = "inline-flex";
  }

  // Rating badge
  if (tmdbData.voteAverage && tmdbData.voteAverage > 0) {
    ratingBadge.innerHTML = `${iconHtml("star-filled", 12)} ${tmdbData.voteAverage.toFixed(1)} / 10`;
    ratingBadge.style.display = "inline-flex";
  }

  // Release year badge
  if (tmdbData.releaseDate && tmdbData.releaseDate.length >= 4) {
    yearBadge.textContent = tmdbData.releaseDate.substring(0, 4);
    yearBadge.style.display = "inline-flex";
  }

  // Title variants badges: 1. app language, 2. English, 3. Original (if present)
  const variants = getModalTitleVariants(tmdbData, currentLang);
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

    badgeEl.style.display = "inline-flex";
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
    variantsRow.style.display = "flex";
    setActiveTitleVariantBadge(initialActiveBadge);
  }

  // TMDB Overview
  if (tmdbData.overview && tmdbData.overview.trim().length > 0) {
    tmdbOverviewText.textContent = tmdbData.overview;
    tmdbOverview.style.display = "block";
  }
}

function getCurrentModalLang() {
  const srcId = currentModalChannel?.sourceId || currentSourceId;
  const currentSrcObj = sources.find((s) => s.id === srcId);
  const countryCode =
    currentModalChannel?.countryCode ||
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

  const fetchIndex = currentModalFetchIndex;
  const switchIndex = ++currentModalLangSwitchIndex;

  tmdbLoading.style.display = "flex";
  try {
    const details = await invoke("get_tmdb_details_for_lang", {
      tmdbId: tmdbData.tmdbId,
      mediaType: tmdbData.mediaType,
      lang: targetLang,
      queryTitle: currentModalTitle,
    });

    if (
      fetchIndex !== currentModalFetchIndex ||
      switchIndex !== currentModalLangSwitchIndex
    ) {
      return;
    }

    tmdbLoading.style.display = "none";

    if (clickedBadgeEl) {
      setActiveTitleVariantBadge(clickedBadgeEl);
    }

    const isCurrentSwitch = () =>
      fetchIndex === currentModalFetchIndex &&
      switchIndex === currentModalLangSwitchIndex;

    if (details.posterPath) {
      currentModalPosterPath = details.posterPath;
      currentModalLargePosterSrc = "";
      const posterUrl = `https://image.tmdb.org/t/p/w500${details.posterPath}`;
      posterCol.style.display = "block";
      updateModalLeftColVisibility();
      loadCachedTmdbImage(posterUrl, isCurrentSwitch, (src) => {
        posterImg.src = src;
      });
      preloadAndCacheLargePoster(details.posterPath);
    }

    if (details.backdropPath) {
      const backdropUrl = `https://image.tmdb.org/t/p/w780${details.backdropPath}`;
      backdrop.style.display = "block";
      loadCachedTmdbImage(backdropUrl, isCurrentSwitch, (src) => {
        backdrop.style.backgroundImage = `url('${src}')`;
      });
    }

    if (details.overview && details.overview.trim().length > 0) {
      tmdbOverviewText.textContent = details.overview;
      tmdbOverview.style.display = "block";
    } else {
      tmdbOverview.style.display = "none";
      tmdbOverviewText.textContent = "";
    }
  } catch (e) {
    if (
      fetchIndex === currentModalFetchIndex &&
      switchIndex === currentModalLangSwitchIndex
    ) {
      tmdbLoading.style.display = "none";
    }
    console.warn("TMDB lang switch error:", e);
  }
}

function openFixOverlay() {
  if (!currentModalTitle) return;
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
    programTitleEl.textContent = currentModalTitle;
    const rawDesc = (currentModalDesc || "").trim();
    if (rawDesc) {
      programDescEl.textContent = rawDesc;
      programDescEl.classList.remove("is-empty");
    } else {
      programDescEl.textContent = t("modal_no_desc");
      programDescEl.classList.add("is-empty");
    }
    programContext.style.display = "block";
  }

  overlay.style.display = "flex";
  searchInput.value = currentModalTitle;
  clearBtn.style.display = searchInput.value ? "block" : "none";
  statusEl.style.display = "none";
  resultsEl.innerHTML = "";

  setTimeout(() => {
    searchInput.focus();
    searchInput.select();
  }, 50);

  triggerFixSearch(currentModalTitle);
}

function closeFixOverlay() {
  const overlay = document.getElementById("modalFixOverlay");
  if (overlay) overlay.style.display = "none";
  const modalContent = document.querySelector("#modalOverlay .modal-content");
  if (modalContent) modalContent.classList.remove("has-fix-open");
  if (fixSearchDebounce) {
    clearTimeout(fixSearchDebounce);
    fixSearchDebounce = null;
  }
}

async function triggerFixSearch(query) {
  const statusEl = document.getElementById("modalFixStatus");
  const resultsEl = document.getElementById("modalFixResultsList");
  const spinner = document.getElementById("modalFixSpinner");

  const q = (query || "").trim();
  if (!q) {
    resultsEl.innerHTML = "";
    statusEl.textContent = t("tmdb_fix_no_results");
    statusEl.style.display = "block";
    return;
  }

  spinner.style.display = "inline-block";
  statusEl.textContent = t("tmdb_fix_searching");
  statusEl.style.display = "block";
  resultsEl.innerHTML = "";

  const lang = getCurrentModalLang();

  try {
    const results = await invoke("search_tmdb", { query: q, lang });
    spinner.style.display = "none";

    if (!results || results.length === 0) {
      statusEl.textContent = t("tmdb_fix_no_results");
      statusEl.style.display = "block";
      return;
    }

    statusEl.style.display = "none";
    renderFixResults(results);
  } catch (e) {
    spinner.style.display = "none";
    const errStr = String(e);
    if (errStr.includes("TMDB API")) {
      statusEl.textContent = t("tmdb_fix_api_key_missing");
    } else {
      statusEl.textContent = errStr;
    }
    statusEl.style.display = "block";
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
          queryTitle: currentModalTitle,
          tmdbId: item.id,
          mediaType: item.mediaType,
          lang,
        });
        renderTmdbDetailsInModal(updatedDetails, { activeLang: lang });
        closeFixOverlay();
        if (window.TraktUI) {
          window.TraktUI.attachToModal({
            title: currentModalTitle,
            tmdbId: updatedDetails?.tmdbId ?? null,
            mediaType: updatedDetails?.mediaType ?? null,
            tmdbData: updatedDetails,
          });
        }
      } catch (err) {
        console.error("Hiba a TMDB egyezés mentésekor:", err);
        card.classList.remove("selecting");
        alert(String(err));
      }
    };

    resultsEl.appendChild(card);
  });
}

async function removeTmdbMatch() {
  if (!currentModalTitle) return;
  const lang = getCurrentModalLang();
  try {
    await invoke("clear_tmdb_match", {
      queryTitle: currentModalTitle,
      lang,
    });
    resetModalTmdbUi();
    closeFixOverlay();
    if (window.TraktUI) {
      window.TraktUI.attachToModal({
        title: currentModalTitle,
        tmdbId: null,
        mediaType: null,
        tmdbData: null,
      });
    }
  } catch (e) {
    console.error("Hiba a TMDB egyezés törlésekor:", e);
  }
}

async function loadTmdbSettings() {
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
      statusBadge.style.background = "#2b6cb0";
    } else {
      keyInput.value = "";
      statusBadge.textContent = t("tmdb_status_not_configured");
      statusBadge.style.background = "#4a5568";
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
      statusBadge.style.background = "#2b6cb0";
      await showAlert(t("alert_success_title"), t("tmdb_saved_success"));
    } else {
      statusBadge.textContent = t("tmdb_status_not_configured");
      statusBadge.style.background = "#4a5568";
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
    statusBadge.style.background = "#4a5568";
    return;
  }
  statusBadge.textContent = "...";
  try {
    const isValid = await invoke("test_tmdb_api_key", { apiKey: key });
    if (isValid) {
      statusBadge.textContent = t("tmdb_status_valid");
      statusBadge.style.background = "#28a745";
    } else {
      statusBadge.textContent = t("tmdb_status_invalid");
      statusBadge.style.background = "#e53e3e";
    }
  } catch (e) {
    console.error("Hiba a TMDB kulcs tesztelésekor:", e);
    statusBadge.textContent = t("tmdb_status_invalid");
    statusBadge.style.background = "#e53e3e";
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
  document.getElementById("modalFixClearBtn").style.display = val
    ? "block"
    : "none";
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
  document.getElementById("modalFixClearBtn").style.display = "none";
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
