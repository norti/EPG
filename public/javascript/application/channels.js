import { state, invoke, LOCALE_COMPARE_OPTIONS } from "./state.js";
import { t, getCurrentLang } from "./i18n.js";
import { Icons } from "../utils/icons.js";
import { SearchUtils } from "../utils/search.js";
import {
  iconHtml,
  fuzzySearchMatch,
  showConfirm,
  showAlert,
  showPrompt,
  openLogoSearchModal,
  escapeHtml,
  appendSafeImage,
  formatByteSize,
  setExportImportStatus,
} from "./ui.js";
import {
  updateHeaderSortDropdown,
  applyActiveSortToEpgChannels,
  fetchEPG,
  renderEPG,
} from "./epg.js";

export async function openMissingLogosModal(sourceId, onStatsChanged) {
  const modal = document.getElementById("missingLogosModal");
  const closeBtn = document.getElementById("missingLogosClose");
  const searchInput = document.getElementById("missingLogosSearchInput");
  const listEl = document.getElementById("missingLogosList");
  const countBadge = document.getElementById("missingLogosCountBadge");

  if (!modal || !listEl) return;

  modal.classList.remove("u-hidden");
  modal.classList.add("active");
  if (searchInput) searchInput.value = "";
  if (countBadge) countBadge.textContent = "…";
  listEl.innerHTML = `<div class="missing-logos-empty"><span class="spinner-small"></span><span>${escapeHtml(t("missing_logos_loading"))}</span></div>`;

  let items = [];
  let isClosed = false;

  const updateCountBadge = () => {
    if (!countBadge) return;
    const total = items.length;
    const assigned = items.filter((it) => !!it.selectedUrl).length;
    if (assigned > 0) {
      countBadge.textContent = t("missing_logos_assigned_count", {
        assigned,
        total,
      });
    } else {
      countBadge.textContent = String(total);
    }
  };

  const renderPreview = (previewEl, item) => {
    if (!previewEl) return;
    previewEl.innerHTML = "";
    if (item.loadingPreview) {
      previewEl.innerHTML = '<span class="spinner-small"></span>';
      return;
    }
    const imgSrc = item.cachedData || item.selectedUrl;
    if (imgSrc) {
      appendSafeImage(previewEl, imgSrc, { onErrorMode: "errSpan" });
    } else {
      const tvSvg = Icons
        ? Icons.get("device-tv", { size: 18 })
        : "TV";
      previewEl.innerHTML = `<span class="logo-preview-placeholder">${tvSvg}</span>`;
    }
  };

  const syncEpgChannelLogo = (item, logoUrl, cachedData) => {
    if (!state.epgData || !state.epgData.channels) return;
    const matchesChannel = (c) =>
      (c.sourceId
        ? c.sourceId === item.sourceId
        : state.currentSourceId === item.sourceId) &&
      (c.id === item.id || c.id === `${item.sourceId}::${item.id}`);

    let touched = false;
    state.epgData.channels.forEach((liveCh) => {
      if (matchesChannel(liveCh)) {
        liveCh.customIcon = logoUrl;
        liveCh.cachedIcon = cachedData || null;
        touched = true;
      }
    });
    if (state.epgData.rawChannels) {
      state.epgData.rawChannels.forEach((rawCh) => {
        if (matchesChannel(rawCh)) {
          rawCh.customIcon = logoUrl;
          rawCh.cachedIcon = cachedData || null;
        }
      });
    }
    if (touched) {
      renderEPG();
    }
  };

  const renderList = () => {
    const query = searchInput ? searchInput.value.trim() : "";
    const preparedQuery =
      query && SearchUtils ? SearchUtils.prepareQuery(query) : null;

    const filtered = items.filter((item) => {
      if (!query) return true;
      if (preparedQuery) {
        return (
          SearchUtils.matchPrepared(item.name, preparedQuery) ||
          SearchUtils.matchPrepared(item.sourceName, preparedQuery)
        );
      }
      return (
        fuzzySearchMatch(item.name, query) ||
        fuzzySearchMatch(item.sourceName, query)
      );
    });

    listEl.innerHTML = "";

    if (items.length === 0) {
      listEl.innerHTML = `<div class="missing-logos-empty">${escapeHtml(t("logos_no_missing"))}</div>`;
      return;
    }

    if (filtered.length === 0) {
      listEl.innerHTML = `<div class="missing-logos-empty">${escapeHtml(t("no_channels_match_filter"))}</div>`;
      return;
    }

    filtered.forEach((item) => {
      const row = document.createElement("div");
      row.className = `missing-logo-item${item.selectedUrl ? " is-assigned" : ""}`;
      row.dataset.key = item.key;

      row.innerHTML = `
        <div class="missing-logo-preview"></div>
        <div class="missing-logo-info">
          <div class="missing-logo-header-line">
            <span class="missing-logo-name" title="${escapeHtml(item.name)}">${escapeHtml(item.name)}</span>
            <span class="channel-order-source-badge" title="${escapeHtml(item.sourceName)}">${escapeHtml(item.sourceName)}</span>
          </div>
          <div class="missing-logo-url${item.selectedUrl ? "" : " u-hidden"}" title="${escapeHtml(item.selectedUrl || "")}">${escapeHtml(item.selectedUrl || "")}</div>
        </div>
        <div class="missing-logo-actions">
          <button type="button" class="btn btn-blue btn-icon-only btn-missing-logo-search" title="${escapeHtml(t("btn_search_logo_title"))}">${iconHtml("photo-search", 15)}</button>
        </div>
      `;

      const previewEl = row.querySelector(".missing-logo-preview");
      const urlEl = row.querySelector(".missing-logo-url");
      const searchBtn = row.querySelector(".btn-missing-logo-search");

      renderPreview(previewEl, item);

      searchBtn.onclick = async (e) => {
        e.stopPropagation();
        const selectedUrl = await openLogoSearchModal(
          item.name,
          item.countryCode || "HU",
        );
        if (!selectedUrl || isClosed) return;

        searchBtn.disabled = true;
        try {
          await invoke("set_channel_logo", {
            sourceId: item.sourceId,
            channelId: item.id,
            logoUrl: selectedUrl,
          });
        } catch (err) {
          searchBtn.disabled = false;
          await showAlert(t("alert_error_title"), String(err));
          return;
        }
        searchBtn.disabled = false;

        item.selectedUrl = selectedUrl;
        item.cachedData = null;
        item.loadingPreview = true;

        row.classList.add("is-assigned");
        if (urlEl) {
          urlEl.textContent = selectedUrl;
          urlEl.title = selectedUrl;
          urlEl.classList.remove("u-hidden");
        }
        renderPreview(previewEl, item);
        updateCountBadge();

        syncEpgChannelLogo(item, selectedUrl, null);
        if (typeof onStatsChanged === "function") {
          onStatsChanged();
        }

        try {
          const dataUri = await invoke("cache_logo", { url: selectedUrl });
          if (item.selectedUrl !== selectedUrl) return;
          item.cachedData = dataUri || null;
          item.loadingPreview = false;
          if (!isClosed && previewEl.isConnected) {
            renderPreview(previewEl, item);
          }
          syncEpgChannelLogo(item, selectedUrl, item.cachedData);
        } catch (_) {
          if (item.selectedUrl !== selectedUrl) return;
          item.loadingPreview = false;
          if (!isClosed && previewEl.isConnected) {
            renderPreview(previewEl, item);
          }
        }
      };

      listEl.appendChild(row);
    });
  };

  const closeModal = () => {
    if (isClosed) return;
    isClosed = true;
    modal.classList.add("u-hidden");
    modal.classList.remove("active");
    modal.onclick = null;
    if (closeBtn) closeBtn.onclick = null;
    if (searchInput) searchInput.oninput = null;
    document.removeEventListener("keydown", onKeyDown);
  };

  const onKeyDown = (e) => {
    if (e.key !== "Escape") return;
    const logoSearchModal = document.getElementById("logoSearchModal");
    if (logoSearchModal && logoSearchModal.classList.contains("active")) return;
    closeModal();
  };

  modal.onclick = (e) => {
    if (e.target === modal) {
      e.stopPropagation();
      closeModal();
    }
  };
  if (closeBtn) closeBtn.onclick = closeModal;
  if (searchInput) searchInput.oninput = renderList;
  document.addEventListener("keydown", onKeyDown);

  try {
    const missingChannels = await invoke("get_missing_logo_channels", {
      sourceId,
    });
    if (isClosed) return;
    items = (missingChannels || []).map((ch) => {
      const srcObj = state.sources.find((s) => s.id === ch.sourceId);
      const sourceName = srcObj ? srcObj.name : ch.sourceName || ch.sourceId;
      const countryCode =
        ch.countryCode || (srcObj ? srcObj.countryCode : null) || "HU";
      return {
        key: `${ch.sourceId}::${ch.id}`,
        id: ch.id,
        sourceId: ch.sourceId,
        name: ch.name,
        sourceName,
        countryCode,
        selectedUrl: "",
        cachedData: null,
        loadingPreview: false,
      };
    });
    updateCountBadge();
    renderList();
  } catch (err) {
    if (isClosed) return;
    if (countBadge) countBadge.textContent = "0";
    listEl.innerHTML = `<div class="missing-logos-empty">${escapeHtml(t("logo_search_error") + " " + err)}</div>`;
  }
}

let isAutoMatchRunning = false;

export async function loadChannelLogosTab() {
  const select = document.getElementById("logoSourceSelect");
  const showMissingBtn = document.getElementById("showMissingLogosBtn");
  const autoMatchBtn = document.getElementById("autoMatchLogosBtn");
  const statsTotal = document.getElementById("logoStatTotal");
  const statsWith = document.getElementById("logoStatWithLogo");
  const statsMissing = document.getElementById("logoStatMissing");
  const progressContainer = document.getElementById("logosProgressContainer");
  const progressFill = document.getElementById("logosProgressFill");
  const statusText = document.getElementById("logosStatusText");
  const resultsContainer = document.getElementById("logosResultsContainer");
  const resultsSummary = document.getElementById("logosResultsSummary");
  const matchesGrid = document.getElementById("logosMatchesGrid");

  if (!select) return;

  const prevVal = select.value;
  select.innerHTML = "";

  const allOpt = document.createElement("option");
  allOpt.value = "__ALL_SOURCES__";
  allOpt.textContent = t("logo_source_all");
  select.appendChild(allOpt);

  state.sources.forEach((s) => {
    const opt = document.createElement("option");
    opt.value = s.id;
    opt.textContent = `${s.name} [${s.countryCode || "HU"}]`;
    select.appendChild(opt);
  });

  if (prevVal && Array.from(select.options).some((o) => o.value === prevVal)) {
    select.value = prevVal;
  } else if (state.currentSourceId && state.currentSourceId !== "__ALL_FAVORITES__") {
    select.value = state.currentSourceId;
  } else {
    select.value = "__ALL_SOURCES__";
  }

  const updateStats = async () => {
    const targetSource =
      select.value === "__ALL_SOURCES__" ? null : select.value;
    try {
      const stats = await invoke("get_channel_logo_stats", {
        sourceId: targetSource,
      });
      statsTotal.textContent = stats.totalChannels;
      statsWith.textContent = stats.withLogoCount;
      statsMissing.textContent = stats.missingLogoCount;
      if (!isAutoMatchRunning) {
        const noMissing = stats.missingLogoCount === 0;
        autoMatchBtn.disabled = noMissing;
        if (showMissingBtn) showMissingBtn.disabled = noMissing;
      }
    } catch (e) {
      console.error("Hiba a statisztika lekérésekor:", e);
    }
  };

  select.onchange = () => {
    if (isAutoMatchRunning) return;
    progressContainer.classList.add("u-hidden");
    resultsContainer.classList.add("u-hidden");
    updateStats();
  };

  if (showMissingBtn) {
    showMissingBtn.onclick = () => {
      if (isAutoMatchRunning) return;
      const targetSource =
        select.value === "__ALL_SOURCES__" ? null : select.value;
      openMissingLogosModal(targetSource, updateStats);
    };
  }

  const setButtonRunningState = (running) => {
    isAutoMatchRunning = running;
    if (running) {
      autoMatchBtn.className = "btn btn-red";
      autoMatchBtn.innerHTML = `<span data-icon="player-stop" data-icon-size="15">${iconHtml("player-stop", 15)}</span><span>${escapeHtml(t("btn_stop_auto_match") || "Leállítás")}</span>`;
      autoMatchBtn.disabled = false;
      select.disabled = true;
      if (showMissingBtn) showMissingBtn.disabled = true;
    } else {
      autoMatchBtn.className = "btn btn-green";
      autoMatchBtn.innerHTML = `<span data-icon="bolt" data-icon-size="15">${iconHtml("bolt", 15)}</span><span data-i18n="btn_auto_match_logos">${escapeHtml(t("btn_auto_match_logos") || "Hiányzó logók automatikus keresése")}</span>`;
      select.disabled = false;
    }
  };

  autoMatchBtn.onclick = async () => {
    if (isAutoMatchRunning) {
      isAutoMatchRunning = false;
      statusText.textContent = "Leállítás folyamatban...";
      autoMatchBtn.disabled = true;
      return;
    }

    const targetSource =
      select.value === "__ALL_SOURCES__" ? null : select.value;

    let missingChannels = [];
    try {
      missingChannels = await invoke("get_missing_logo_channels", {
        sourceId: targetSource,
      });
    } catch (err) {
      alert("Hiba a csatornák lekérésekor: " + err);
      return;
    }

    if (!missingChannels || missingChannels.length === 0) {
      await showAlert(t("alert_info_title") || "Info", t("logos_no_missing"));
      return;
    }

    setButtonRunningState(true);
    progressContainer.classList.remove("u-hidden");
    progressFill.style.width = "0%";
    statusText.textContent =
      t("logos_preloading") ||
      "Adatforrások gyorstárazása (tv-logo, iptv-org)...";

    // Rejtjük el az előnézeti rácsot a keresés idejére
    resultsContainer.classList.add("u-hidden");
    resultsSummary.textContent = "";
    matchesGrid.innerHTML = "";

    // Aktuális számláló értékek beolvasása a valós idejű léptetéshez
    let currentWithLogo = parseInt(statsWith.textContent, 10) || 0;
    let currentMissing =
      parseInt(statsMissing.textContent, 10) || missingChannels.length;

    // 1. LÉPÉS: Források fájllistájának / fanézetének előzetes gyorstárazása a kereséshez
    try {
      const uniqueCountries = Array.from(
        new Set(missingChannels.map((c) => c.countryCode || "HU")),
      );
      await invoke("preload_logo_sources", { countryCodes: uniqueCountries });
    } catch (preloadErr) {
      console.warn(
        "Figyelmeztetés az adatforrások gyorstárazásakor:",
        preloadErr,
      );
    }

    let matchedList = [];
    let round2Channels = [];
    let failedCount = 0;
    let wasStopped = false;

    const includeWikiCheck = document.getElementById(
      "logoIncludeWikimediaCheck",
    );
    const includeWiki = includeWikiCheck ? includeWikiCheck.checked : true;

    // 2. LÉPÉS: 1. KÖR - Keresés a gyorstárazott GitHub forrásokban (tv-logo és iptv-org)
    for (let i = 0; i < missingChannels.length; i++) {
      if (!isAutoMatchRunning) {
        wasStopped = true;
        break;
      }

      const ch = missingChannels[i];
      const percent = Math.round((i / missingChannels.length) * 100);
      progressFill.style.width = percent + "%";

      const searchMsg = (
        t("logos_round1_msg") ||
        "1. Kör (GitHub): [{current}/{total}] {channel} keresése..."
      )
        .replace("{current}", i + 1)
        .replace("{total}", missingChannels.length)
        .replace("{channel}", ch.name);
      statusText.textContent = searchMsg;

      try {
        const match = await invoke("find_channel_logo_github", {
          sourceId: ch.sourceId,
          channelId: ch.id,
          channelName: ch.name,
          countryCode: ch.countryCode,
        });

        if (match) {
          matchedList.push(match);

          // Minden megtalált logó után azonnal frissíti a számlálókat!
          currentWithLogo++;
          currentMissing = Math.max(0, currentMissing - 1);
          statsWith.textContent = currentWithLogo;
          statsMissing.textContent = currentMissing;

          const foundMsg = (
            t("logos_found_channel") ||
            "[{current}/{total}] {channel} → Találat ({source})"
          )
            .replace("{current}", i + 1)
            .replace("{total}", missingChannels.length)
            .replace("{channel}", ch.name)
            .replace("{source}", match.sourceName);
          statusText.textContent = foundMsg;
        } else {
          round2Channels.push(ch);
        }
      } catch (err) {
        console.error("Hiba a csatorna keresésekor (GitHub):", ch.name, err);
        round2Channels.push(ch);
      }
    }

    // 2.b LÉPÉS: 2. KÖR - Online keresés a Wikimédián a GitHubon nem talált logókhoz
    if (round2Channels.length > 0 && includeWiki && isAutoMatchRunning) {
      statusText.textContent = (
        t("logos_round2_start") ||
        "2. Kör: Online keresés a Wikimédián a maradék {count} csatornához..."
      ).replace("{count}", round2Channels.length);
      progressFill.style.width = "0%";

      for (let k = 0; k < round2Channels.length; k++) {
        if (!isAutoMatchRunning) {
          wasStopped = true;
          break;
        }

        const ch = round2Channels[k];
        const percent2 = Math.round((k / round2Channels.length) * 100);
        progressFill.style.width = percent2 + "%";

        const searchMsg2 = (
          t("logos_round2_msg") ||
          "2. Kör (Wikimedia): [{current}/{total}] {channel} keresése..."
        )
          .replace("{current}", k + 1)
          .replace("{total}", round2Channels.length)
          .replace("{channel}", ch.name);
        statusText.textContent = searchMsg2;

        try {
          const match = await invoke("find_channel_logo_wikimedia", {
            sourceId: ch.sourceId,
            channelId: ch.id,
            channelName: ch.name,
            countryCode: ch.countryCode,
          });

          if (match) {
            matchedList.push(match);

            currentWithLogo++;
            currentMissing = Math.max(0, currentMissing - 1);
            statsWith.textContent = currentWithLogo;
            statsMissing.textContent = currentMissing;

            const foundMsg = (
              t("logos_found_channel") ||
              "[{current}/{total}] {channel} → Találat ({source})"
            )
              .replace("{current}", k + 1)
              .replace("{total}", round2Channels.length)
              .replace("{channel}", ch.name)
              .replace("{source}", match.sourceName);
            statusText.textContent = foundMsg;
          } else {
            failedCount++;
          }
        } catch (err) {
          console.error(
            "Hiba a csatorna keresésekor (Wikimedia):",
            ch.name,
            err,
          );
          failedCount++;
        }
      }
    } else if (round2Channels.length > 0) {
      failedCount += round2Channels.length;
    }

    // 3. LÉPÉS: Amint végzett a kereséssel, letölti és gyorstárazza a logókat
    if (matchedList.length > 0 && isAutoMatchRunning) {
      statusText.textContent = (
        t("logos_search_phase_done") ||
        "Keresés kész. {count} logó letöltése folyamatban..."
      ).replace("{count}", matchedList.length);
      progressFill.style.width = "0%";

      for (let j = 0; j < matchedList.length; j++) {
        if (!isAutoMatchRunning) {
          wasStopped = true;
          break;
        }

        const item = matchedList[j];
        const dlPercent = Math.round(((j + 1) / matchedList.length) * 100);
        progressFill.style.width = dlPercent + "%";

        const dlMsg = (
          t("logos_downloading_progress") ||
          "Logók letöltése: [{current}/{total}] {channel}..."
        )
          .replace("{current}", j + 1)
          .replace("{total}", matchedList.length)
          .replace("{channel}", item.channelName);
        statusText.textContent = dlMsg;

        try {
          const dataUri = await invoke("cache_logo", { url: item.logoUrl });
          item.cachedData = dataUri;
        } catch (dlErr) {
          console.warn("Hiba a logó letöltésekor:", item.logoUrl, dlErr);
        }
      }
    }

    // 4. LÉPÉS: Csak a teljes folyamat befejezése után listázza ki a megtalált logókat
    resultsContainer.classList.remove("u-hidden");
    matchesGrid.innerHTML = "";

    matchedList.forEach((m) => {
      const itemEl = document.createElement("div");
      itemEl.className = "logo-match-item";
      itemEl.innerHTML = `
        <div class="logo-match-thumb"></div>
        <div class="logo-match-info">
          <div class="logo-match-name" title="${escapeHtml(m.channelName)}">${escapeHtml(m.channelName)}</div>
          <div class="logo-match-source">${escapeHtml(m.sourceName)}</div>
        </div>
      `;
      appendSafeImage(
        itemEl.querySelector(".logo-match-thumb"),
        m.cachedData || m.logoUrl,
        { onErrorMode: "errText" },
      );
      matchesGrid.appendChild(itemEl);
    });

    // Frissítjük a memóriában lévő EPG adatokat és újrarajzoljuk az EPG-t
    if (state.epgData && state.epgData.channels && matchedList.length > 0) {
      matchedList.forEach((m) => {
        const liveCh = state.epgData.channels.find(
          (c) => c.id === m.channelId || c.id.endsWith("::" + m.channelId),
        );
        if (liveCh) {
          liveCh.customIcon = m.logoUrl;
          liveCh.cachedIcon = m.cachedData || m.logoUrl;
        }
        if (state.epgData.rawChannels) {
          const rawCh = state.epgData.rawChannels.find(
            (c) => c.id === m.channelId || c.id.endsWith("::" + m.channelId),
          );
          if (rawCh) {
            rawCh.customIcon = m.logoUrl;
            rawCh.cachedIcon = m.cachedData || m.logoUrl;
          }
        }
      });
      renderEPG();
    }

    progressFill.style.width = "100%";
    if (wasStopped) {
      statusText.textContent = (
        t("logos_status_stopped") ||
        "Keresés megállítva. Megtalált logók: {count} db."
      ).replace("{count}", matchedList.length);
    } else {
      statusText.textContent = t("status_ready") || "Kész!";
    }

    setButtonRunningState(false);

    resultsSummary.textContent = (
      t("logos_search_summary") ||
      "Vizsgált hiányzó csatornák: {missing}. Sikeresen megtalálva: {matched}, sikertelen: {failed}."
    )
      .replace(
        "{missing}",
        wasStopped ? matchedList.length + failedCount : missingChannels.length,
      )
      .replace("{matched}", matchedList.length)
      .replace("{failed}", failedCount);

    if (matchedList.length === 0 && !wasStopped) {
      matchesGrid.innerHTML = `<p class="empty-list-message empty-list-message--grid">${t("logo_search_no_results")}</p>`;
    }

    await updateStats();
  };

  setupLogoDeduplicationHandlers();
  await updateStats();
}

let isLogoDedupSetup = false;
function setupLogoDeduplicationHandlers() {
  const findBtn = document.getElementById("findDuplicateLogosBtn");
  const dedupBtn = document.getElementById("deduplicateLogosBtn");
  const statusEl = document.getElementById("logoDedupStatus");
  const resultsEl = document.getElementById("logoDedupResults");
  const summaryEl = document.getElementById("logoDedupSummary");
  const listEl = document.getElementById("logoDedupList");

  if (!findBtn || !dedupBtn || isLogoDedupSetup) return;
  isLogoDedupSetup = true;

  findBtn.onclick = async () => {
    findBtn.disabled = true;
    dedupBtn.classList.add("u-hidden");
    resultsEl.classList.add("u-hidden");
    setExportImportStatus(
      "logoDedupStatus",
      t("logo_dedup_searching"),
      "loading",
    );

    try {
      const report = await invoke("find_duplicate_logos");
      if (!report.duplicateGroups || report.duplicateGroups.length === 0) {
        setExportImportStatus(
          "logoDedupStatus",
          t("logo_dedup_no_duplicates"),
          "success",
        );
        summaryEl.textContent = t("logo_dedup_no_duplicates");
        listEl.innerHTML = "";
        resultsEl.classList.remove("u-hidden");
      } else {
        const foundMsg = t("logo_dedup_found")
          .replace("{count}", report.redundantCount)
          .replace("{groups}", report.duplicateGroups.length)
          .replace("{size}", formatByteSize(report.potentialSavedBytes));

        setExportImportStatus("logoDedupStatus", foundMsg, "success");
        summaryEl.textContent = foundMsg;
        dedupBtn.classList.remove("u-hidden");

        listEl.innerHTML = "";
        report.duplicateGroups.forEach((g) => {
          const groupDiv = document.createElement("div");
          groupDiv.className = "logo-dedup-group";

          const channelsLabel =
            g.channels && g.channels.length > 0 ? g.channels.join(", ") : "N/A";

          groupDiv.innerHTML = `
            <div class="logo-dedup-thumb"></div>
            <div class="logo-dedup-details">
              <div class="logo-dedup-channels">
                ${escapeHtml(channelsLabel)}
                <span class="logo-dedup-badge">${g.urls.length} URLs</span>
                <span class="logo-dedup-badge">${escapeHtml(formatByteSize(g.byteSize))}</span>
              </div>
              <div class="logo-dedup-urls">
                ${g.urls.map((u) => `<span>${iconHtml("link", 13)} ${escapeHtml(u)}</span>`).join("")}
              </div>
            </div>
          `;
          appendSafeImage(
            groupDiv.querySelector(".logo-dedup-thumb"),
            g.sampleData,
            { onErrorMode: "errText" },
          );
          listEl.appendChild(groupDiv);
        });
        resultsEl.classList.remove("u-hidden");
      }
    } catch (err) {
      console.error(err);
      setExportImportStatus(
        "logoDedupStatus",
        t("logo_dedup_error") + err,
        "error",
      );
    } finally {
      findBtn.disabled = false;
    }
  };

  dedupBtn.onclick = async () => {
    const confirmed = await showConfirm(
      t("btn_deduplicate_logos"),
      t("logo_dedup_confirm"),
    );
    if (!confirmed) return;

    dedupBtn.disabled = true;
    findBtn.disabled = true;
    setExportImportStatus(
      "logoDedupStatus",
      t("logo_dedup_optimizing"),
      "loading",
    );

    try {
      const res = await invoke("deduplicate_logos");
      const successMsg = t("logo_dedup_success")
        .replace("{removedUrls}", res.removedUrls)
        .replace("{removedImages}", res.removedImages)
        .replace("{freedSize}", formatByteSize(res.freedBytes));

      setExportImportStatus("logoDedupStatus", successMsg, "success");
      await showAlert(t("alert_success_title"), successMsg);
      resultsEl.classList.add("u-hidden");
      dedupBtn.classList.add("u-hidden");

      await fetchEPG();
      renderEPG();
    } catch (err) {
      console.error(err);
      setExportImportStatus(
        "logoDedupStatus",
        t("logo_dedup_error") + err,
        "error",
      );
      await showAlert(t("alert_error_title"), err);
    } finally {
      findBtn.disabled = false;
      dedupBtn.disabled = false;
    }
  };
}

let currentSettingsPresets = [];

export function updateSortUiForSource() {
  const select = document.getElementById("channelOrderSourceSelect");
  const targetSourceId = select ? select.value : state.currentSourceId;
  const sourceAzBtn = document.getElementById("sortChannelsSourceAzBtn");
  if (sourceAzBtn) {
    sourceAzBtn.classList.toggle(
      "u-hidden",
      targetSourceId !== "__ALL_FAVORITES__",
    );
  }
}

export async function loadChannelSortPresetsDropdown() {
  const sourceSelect = document.getElementById("channelOrderSourceSelect");
  const targetSourceId = sourceSelect ? sourceSelect.value : state.currentSourceId;
  const presetSelect = document.getElementById("channelSortPresetSelect");
  const renameBtn = document.getElementById("renameChannelSortPresetBtn");
  const deleteBtn = document.getElementById("deleteChannelSortPresetBtn");
  if (!presetSelect) return;

  presetSelect.innerHTML = `<option value="">${t("sort_preset_default")}</option>`;
  if (renameBtn) renameBtn.disabled = true;
  if (deleteBtn) deleteBtn.disabled = true;

  if (!targetSourceId) return;

  try {
    currentSettingsPresets = await invoke("get_sort_presets", {
      sourceId: targetSourceId,
    });
    currentSettingsPresets.forEach((p) => {
      const opt = document.createElement("option");
      opt.value = p.id;
      opt.textContent = p.name;
      presetSelect.appendChild(opt);
    });
  } catch (err) {
    console.error("Failed to load sort presets:", err);
  }
}

export function loadChannelOrderSourcesDropdown() {
  const select = document.getElementById("channelOrderSourceSelect");
  if (!select) return;
  const prevVal = select.value;
  select.innerHTML = "";
  if (state.sources.length > 0) {
    const allFavOpt = document.createElement("option");
    allFavOpt.value = "__ALL_FAVORITES__";
    allFavOpt.textContent = t("all_favorites_source");
    select.appendChild(allFavOpt);
  }
  state.sources.forEach((s) => {
    const opt = document.createElement("option");
    opt.value = s.id;
    opt.textContent = `${s.name} [${s.countryCode || "HU"}]`;
    select.appendChild(opt);
  });
  if (prevVal && Array.from(select.options).some((o) => o.value === prevVal)) {
    select.value = prevVal;
  } else if (state.currentSourceId) {
    select.value = state.currentSourceId;
  } else if (state.sources.length > 0) {
    select.value = "__ALL_FAVORITES__";
  }
  select.onchange = () => {
    updateSortUiForSource();
    loadChannelSortPresetsDropdown();
    renderChannelOrderList();
  };
  updateSortUiForSource();
}

function initChannelSortHandlers() {
const presetSelectEl = document.getElementById("channelSortPresetSelect");
if (presetSelectEl) {
  presetSelectEl.onchange = async () => {
    const presetId = parseInt(presetSelectEl.value, 10);
    const renameBtn = document.getElementById("renameChannelSortPresetBtn");
    const deleteBtn = document.getElementById("deleteChannelSortPresetBtn");
    if (renameBtn) renameBtn.disabled = !presetSelectEl.value;
    if (deleteBtn) deleteBtn.disabled = !presetSelectEl.value;

    if (!presetId) return;
    const preset = currentSettingsPresets.find((p) => p.id === presetId);
    if (!preset || !preset.channelOrder) return;

    const container = document.getElementById("channelOrderList");
    if (!container) return;
    const items = Array.from(container.querySelectorAll(".channel-order-item"));
    const order = preset.channelOrder;

    items.sort((a, b) => {
      const idA = a.dataset.channelId;
      const idB = b.dataset.channelId;
      const idxA = order.indexOf(idA);
      const idxB = order.indexOf(idB);
      if (idxA === -1 && idxB === -1) return 0;
      if (idxA === -1) return 1;
      if (idxB === -1) return -1;
      return idxA - idxB;
    });

    items.forEach((item, idx) => {
      const idxEl = item.querySelector(".order-index");
      if (idxEl) idxEl.textContent = idx + 1;
      container.appendChild(item);
    });

    const select = document.getElementById("channelOrderSourceSelect");
    const targetSourceId = select ? select.value : state.currentSourceId;
    const newOrder = items.map((el) => el.dataset.channelId);

    await invoke("save_channel_order", {
      sourceId: targetSourceId,
      order: newOrder,
    });

    if (targetSourceId === state.currentSourceId) {
      if (state.epgData.rawChannels) {
        state.epgData.rawChannels.sort((a, b) => {
          if (
            state.currentSourceId !== "__ALL_FAVORITES__" &&
            a.isFavorite !== b.isFavorite
          ) {
            return a.isFavorite ? -1 : 1;
          }
          return newOrder.indexOf(a.id) - newOrder.indexOf(b.id);
        });
      }
      applyActiveSortToEpgChannels();
      renderEPG();
    }
  };
}

const savePresetBtn = document.getElementById("saveChannelSortPresetBtn");
if (savePresetBtn) {
  savePresetBtn.onclick = async () => {
    const sourceSelect = document.getElementById("channelOrderSourceSelect");
    const targetSourceId = sourceSelect ? sourceSelect.value : state.currentSourceId;
    if (!targetSourceId) return;

    const container = document.getElementById("channelOrderList");
    const items = container
      ? Array.from(container.querySelectorAll(".channel-order-item"))
      : [];
    if (items.length === 0) return;

    const name = await showPrompt(
      t("prompt_save_preset_title"),
      t("prompt_save_preset_msg"),
    );
    if (!name) return;

    const order = items.map((el) => el.dataset.channelId);
    try {
      const newPreset = await invoke("save_sort_preset", {
        sourceId: targetSourceId,
        name,
        order,
      });
      await loadChannelSortPresetsDropdown();
      const presetSelect = document.getElementById("channelSortPresetSelect");
      if (presetSelect) {
        presetSelect.value = newPreset.id;
        const renameBtn = document.getElementById("renameChannelSortPresetBtn");
        const deleteBtn = document.getElementById("deleteChannelSortPresetBtn");
        if (renameBtn) renameBtn.disabled = false;
        if (deleteBtn) deleteBtn.disabled = false;
      }
      if (targetSourceId === state.currentSourceId) {
        await updateHeaderSortDropdown();
      }
    } catch (err) {
      await showAlert(t("alert_error_title"), String(err));
    }
  };
}

const renamePresetBtn = document.getElementById("renameChannelSortPresetBtn");
if (renamePresetBtn) {
  renamePresetBtn.onclick = async () => {
    const presetSelect = document.getElementById("channelSortPresetSelect");
    const presetId = parseInt(presetSelect.value, 10);
    if (!presetId) return;
    const preset = currentSettingsPresets.find((p) => p.id === presetId);
    if (!preset) return;

    const newName = await showPrompt(
      t("prompt_rename_preset_title"),
      t("prompt_rename_preset_msg"),
      preset.name,
    );
    if (!newName || newName === preset.name) return;

    try {
      await invoke("rename_sort_preset", {
        presetId,
        newName,
      });
      preset.name = newName;
      await loadChannelSortPresetsDropdown();
      if (presetSelect) presetSelect.value = presetId;
      const targetSourceId = document.getElementById(
        "channelOrderSourceSelect",
      ).value;
      if (targetSourceId === state.currentSourceId) {
        await updateHeaderSortDropdown();
      }
    } catch (err) {
      await showAlert(t("alert_error_title"), String(err));
    }
  };
}

const deletePresetBtn = document.getElementById("deleteChannelSortPresetBtn");
if (deletePresetBtn) {
  deletePresetBtn.onclick = async () => {
    const presetSelect = document.getElementById("channelSortPresetSelect");
    const presetId = parseInt(presetSelect.value, 10);
    if (!presetId) return;
    const preset = currentSettingsPresets.find((p) => p.id === presetId);
    if (!preset) return;

    const confirmed = await showConfirm(
      t("confirm_delete_preset_title"),
      t("confirm_delete_preset_msg", { name: preset.name }),
    );
    if (!confirmed) return;

    try {
      await invoke("delete_sort_preset", { presetId });
      await loadChannelSortPresetsDropdown();
      const targetSourceId = document.getElementById(
        "channelOrderSourceSelect",
      ).value;
      if (targetSourceId === state.currentSourceId) {
        await updateHeaderSortDropdown();
      }
    } catch (err) {
      await showAlert(t("alert_error_title"), String(err));
    }
  };
}

const sortAzBtn = document.getElementById("sortChannelsAzBtn");
if (sortAzBtn) {
  sortAzBtn.onclick = async () => {
    const container = document.getElementById("channelOrderList");
    if (!container) return;
    const items = Array.from(container.querySelectorAll(".channel-order-item"));
    if (items.length === 0) return;

    items.sort((a, b) => {
      const nameA = a.querySelector(".channel-order-name")?.textContent || "";
      const nameB = b.querySelector(".channel-order-name")?.textContent || "";
      return nameA.localeCompare(
        nameB,
        getCurrentLang() === "hungarian" ? "hu" : "en",
        LOCALE_COMPARE_OPTIONS,
      );
    });

    items.forEach((item, idx) => {
      const idxEl = item.querySelector(".order-index");
      if (idxEl) idxEl.textContent = idx + 1;
      container.appendChild(item);
    });

    const select = document.getElementById("channelOrderSourceSelect");
    const targetSourceId = select ? select.value : state.currentSourceId;
    const newOrder = items.map((el) => el.dataset.channelId);

    await invoke("save_channel_order", {
      sourceId: targetSourceId,
      order: newOrder,
    });

    if (targetSourceId === state.currentSourceId) {
      if (state.epgData.rawChannels) {
        state.epgData.rawChannels.sort((a, b) => {
          if (
            state.currentSourceId !== "__ALL_FAVORITES__" &&
            a.isFavorite !== b.isFavorite
          ) {
            return a.isFavorite ? -1 : 1;
          }
          return newOrder.indexOf(a.id) - newOrder.indexOf(b.id);
        });
      }
      applyActiveSortToEpgChannels();
      renderEPG();
    }
  };
}

const sortSourceAzBtn = document.getElementById("sortChannelsSourceAzBtn");
if (sortSourceAzBtn) {
  sortSourceAzBtn.onclick = async () => {
    const container = document.getElementById("channelOrderList");
    if (!container) return;
    const items = Array.from(container.querySelectorAll(".channel-order-item"));
    if (items.length === 0) return;

    items.sort((a, b) => {
      const srcA = a.dataset.sourceName || "";
      const srcB = b.dataset.sourceName || "";
      const srcCmp = srcA.localeCompare(
        srcB,
        getCurrentLang() === "hungarian" ? "hu" : "en",
        LOCALE_COMPARE_OPTIONS,
      );
      if (srcCmp !== 0) return srcCmp;

      const nameA = a.querySelector(".channel-order-name")?.textContent || "";
      const nameB = b.querySelector(".channel-order-name")?.textContent || "";
      return nameA.localeCompare(
        nameB,
        getCurrentLang() === "hungarian" ? "hu" : "en",
        LOCALE_COMPARE_OPTIONS,
      );
    });

    items.forEach((item, idx) => {
      const idxEl = item.querySelector(".order-index");
      if (idxEl) idxEl.textContent = idx + 1;
      container.appendChild(item);
    });

    const select = document.getElementById("channelOrderSourceSelect");
    const targetSourceId = select ? select.value : state.currentSourceId;
    const newOrder = items.map((el) => el.dataset.channelId);

    await invoke("save_channel_order", {
      sourceId: targetSourceId,
      order: newOrder,
    });

    if (targetSourceId === state.currentSourceId) {
      if (state.epgData.rawChannels) {
        state.epgData.rawChannels.sort((a, b) => {
          if (
            state.currentSourceId !== "__ALL_FAVORITES__" &&
            a.isFavorite !== b.isFavorite
          ) {
            return a.isFavorite ? -1 : 1;
          }
          return newOrder.indexOf(a.id) - newOrder.indexOf(b.id);
        });
      }
      applyActiveSortToEpgChannels();
      renderEPG();
    }
  };
}
}

function setupChannelOrderPointerDrag(item, handle, container, updateAndSave) {
  handle.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    item.classList.add("dragging");
    try {
      handle.setPointerCapture(e.pointerId);
    } catch (err) {}

    const onPointerMove = (moveEvent) => {
      const targetEl = document
        .elementFromPoint(moveEvent.clientX, moveEvent.clientY)
        ?.closest(".channel-order-item");
      if (
        targetEl &&
        targetEl !== item &&
        targetEl.parentElement === container
      ) {
        const rect = targetEl.getBoundingClientRect();
        const midY = rect.top + rect.height / 2;
        if (moveEvent.clientY < midY) {
          container.insertBefore(item, targetEl);
        } else {
          container.insertBefore(item, targetEl.nextSibling);
        }
      }
    };

    const onPointerUp = async (upEvent) => {
      document.removeEventListener("pointermove", onPointerMove);
      document.removeEventListener("pointerup", onPointerUp);
      document.removeEventListener("pointercancel", onPointerUp);

      try {
        handle.releasePointerCapture(upEvent.pointerId);
      } catch (err) {}

      item.classList.remove("dragging");
      await updateAndSave();
    };

    document.addEventListener("pointermove", onPointerMove);
    document.addEventListener("pointerup", onPointerUp);
    document.addEventListener("pointercancel", onPointerUp);
  });
}

let channelFilterMode = "all";
let channelSearchQuery = "";

function initChannelFilterHandlers() {
const channelSearchInput = document.getElementById("channelOrderSearchInput");
if (channelSearchInput) {
  channelSearchInput.oninput = (e) => {
    channelSearchQuery = e.target.value.toLowerCase().trim();
    renderChannelOrderList();
  };
}

["all", "active", "fav", "hidden"].forEach((mode) => {
  const btnId =
    mode === "all"
      ? "filterChanAll"
      : mode === "active"
        ? "filterChanActive"
        : mode === "fav"
          ? "filterChanFav"
          : "filterChanHidden";
  const btn = document.getElementById(btnId);
  if (btn) {
    btn.onclick = () => {
      channelFilterMode = mode;
      document
        .querySelectorAll(".btn-filter-channel")
        .forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      renderChannelOrderList();
    };
  }
});
}

export async function renderChannelOrderList() {
  const select = document.getElementById("channelOrderSourceSelect");
  const targetSourceId =
    select?.value ||
    (state.currentSourceId && state.currentSourceId !== "__ALL_FAVORITES__"
      ? state.currentSourceId
      : state.sources.length > 0
        ? state.sources[0].id
        : "");
  const container = document.getElementById("channelOrderList");
  const contentArea = document.querySelector(".settings-content-area");
  if (!container) return;

  if (!targetSourceId) {
    container.innerHTML = `<p class="empty-list-message">${t("no_sources_available")}</p>`;
    return;
  }

  let sourceChannels = [];
  if (
    targetSourceId === state.currentSourceId &&
    state.epgData &&
    state.epgData.channels &&
    state.epgData.channels.length > 0
  ) {
    sourceChannels = state.epgData.rawChannels || state.epgData.channels;
  } else {
    try {
      const res = await invoke("get_epg_data", {
        sourceId: targetSourceId,
      });
      sourceChannels = res.channels || [];
    } catch (err) {
      console.error("Failed to load channels for order list:", err);
      sourceChannels = [];
    }
  }

  const savedContainerScroll = container.scrollTop;
  const savedContentAreaScroll = contentArea ? contentArea.scrollTop : 0;
  container.innerHTML = "";

  if (sourceChannels.length === 0) {
    container.innerHTML = `<p class="empty-list-message">${t("no_channels_in_source")}</p>`;
    return;
  }

  const channelOrderQuery =
    channelSearchQuery && SearchUtils
      ? SearchUtils.prepareQuery(channelSearchQuery)
      : null;

  const filteredChannels = sourceChannels.filter((ch) => {
    if (channelFilterMode === "active" && ch.isHidden) return false;
    if (channelFilterMode === "fav" && !ch.isFavorite) return false;
    if (channelFilterMode === "hidden" && !ch.isHidden) return false;
    if (channelSearchQuery) {
      const nameMatch = channelOrderQuery
        ? SearchUtils.matchPrepared(ch.name, channelOrderQuery)
        : fuzzySearchMatch(ch.name, channelSearchQuery);
      const idMatch = String(ch.id || "")
        .toLowerCase()
        .includes(channelSearchQuery);
      if (!nameMatch && !idMatch) return false;
    }
    return true;
  });

  if (filteredChannels.length === 0) {
    container.innerHTML = `<p class="empty-list-message empty-list-message--padded">${t("no_channels_match_filter")}</p>`;
    return;
  }

  const updateAndSave = async () => {
    const reorderedItems = Array.from(
      container.querySelectorAll(".channel-order-item"),
    );
    let newOrder = [];
    if (channelFilterMode === "all" && !channelSearchQuery) {
      newOrder = reorderedItems.map((el, i) => {
        const idxEl = el.querySelector(".order-index");
        if (idxEl) idxEl.textContent = i + 1;
        return el.dataset.channelId;
      });
    } else {
      newOrder = sourceChannels.map((c) => c.id);
    }

    await invoke("save_channel_order", {
      sourceId: targetSourceId,
      order: newOrder,
    });

    if (targetSourceId === state.currentSourceId) {
      if (state.epgData.rawChannels) {
        state.epgData.rawChannels.sort((a, b) => {
          if (
            state.currentSourceId !== "__ALL_FAVORITES__" &&
            a.isFavorite !== b.isFavorite
          ) {
            return a.isFavorite ? -1 : 1;
          }
          return newOrder.indexOf(a.id) - newOrder.indexOf(b.id);
        });
      }
      applyActiveSortToEpgChannels();
      renderEPG();
    }
  };

  filteredChannels.forEach((ch, idx) => {
    const actualSourceId =
      ch.sourceId ||
      (ch.id.includes("::") ? ch.id.split("::")[0] : targetSourceId);
    const actualChannelId = ch.id.includes("::")
      ? ch.id.split("::").slice(1).join("::")
      : ch.id;

    const item = document.createElement("div");
    const isFav = !!ch.isFavorite;
    const isHidden = !!ch.isHidden;
    item.className = `channel-order-item ${isFav ? "is-fav" : ""} ${isHidden ? "is-hidden" : ""}`;
    item.dataset.channelId = ch.id;
    item.dataset.sourceId = actualSourceId;
    item.dataset.sourceName = ch.sourceName || ch.countryCode || actualSourceId;

    const effectiveLogo = ch.customIcon || ch.icon || "";
    const sourceBadgeText = ch.sourceName || ch.countryCode || actualSourceId;
    const sourceBadgeHtml =
      targetSourceId === "__ALL_FAVORITES__" || ch.sourceName
        ? `<span class="channel-order-source-badge" title="${escapeHtml(sourceBadgeText)}">${escapeHtml(sourceBadgeText)}</span>`
        : "";

    const tvPlaceholderSvg = Icons
      ? Icons.get("device-tv", { size: 18 })
      : "TV";
    item.innerHTML = `
            <div class="channel-item-main">
              <span class="drag-handle" title="${escapeHtml(t("channels_drag_hint"))}">${iconHtml("grip-vertical", 16)}</span>
              <span class="order-index">${idx + 1}</span>
              <div class="channel-order-logo-preview">
                ${effectiveLogo ? "" : `<span class="logo-preview-placeholder">${tvPlaceholderSvg}</span>`}
              </div>
              <span class="channel-order-name" title="${escapeHtml(ch.name)}">${escapeHtml(ch.name)}</span>
              ${sourceBadgeHtml}
              <span class="channel-tag">ID: ${escapeHtml(actualChannelId)}</span>
              ${isHidden ? `<span class="badge-hidden">${escapeHtml(t("channel_hidden_badge"))}</span>` : ""}
              <button class="btn-channel-fav ${isFav ? "active" : ""}" title="${escapeHtml(isFav ? t("channel_fav_toggle_on") : t("channel_fav_toggle_off"))}">${iconHtml(isFav ? "star-filled" : "star", 17)}</button>
              <button class="btn-channel-hide ${isHidden ? "is-hidden" : ""}" title="${escapeHtml(isHidden ? t("channel_unhide_toggle") : t("channel_hide_toggle"))}">
                ${iconHtml(isHidden ? "eye-off" : "eye", 16)}
              </button>
              <div class="channel-reorder-btns">
                <button class="btn-order-arrow btn-up" title="Feljebb">${iconHtml("caret-up-filled", 13)}</button>
                <button class="btn-order-arrow btn-down" title="Lejjebb">${iconHtml("caret-down-filled", 13)}</button>
              </div>
            </div>
            <div class="channel-edit-fields-container">
              <div class="channel-rename-row">
                <span class="channel-rename-label">${escapeHtml(t("channel_name_label"))}</span>
                <input type="text" class="channel-rename-input" placeholder="${escapeHtml(t("channel_name_placeholder"))}" value="${escapeHtml(ch.customName || "")}" />
                <div class="channel-edit-fields-actions">
                  <button class="btn-logo-action btn-channel-data-save btn btn-green">${escapeHtml(t("btn_rename_save"))}</button>
                  ${ch.customName || ch.customIcon ? `<button class="btn-logo-action btn-channel-data-reset btn btn-red btn-icon-only" title="${escapeHtml(t("btn_rename_reset"))}">${iconHtml("x", 13)}</button>` : ""}
                </div>
              </div>
              <div class="channel-logo-row">
                <span class="channel-logo-label">${escapeHtml(t("channel_logo_label"))}</span>
                <div class="channel-edit-input-group">
                  <input type="text" class="channel-logo-input" placeholder="${escapeHtml(t("channel_logo_placeholder"))}" value="${escapeHtml(ch.customIcon || "")}" />
                  <button type="button" class="btn btn-blue btn-icon-only btn-channel-logo-search" title="${escapeHtml(t("btn_search_logo_title"))}">${iconHtml("photo-search", 15)}</button>
                </div>
              </div>
            </div>
          `;
    if (effectiveLogo) {
      appendSafeImage(
        item.querySelector(".channel-order-logo-preview"),
        effectiveLogo,
      );
    }

    // Kedvenc kapcsoló
    item.querySelector(".btn-channel-fav").onclick = async (e) => {
      e.stopPropagation();
      const newFav = await invoke("toggle_channel_favorite", {
        sourceId: actualSourceId,
        channelId: actualChannelId,
      });
      ch.isFavorite = newFav;
      if (targetSourceId === "__ALL_FAVORITES__" && !newFav) {
        sourceChannels = sourceChannels.filter((c) => c.id !== ch.id);
      }
      if (state.currentSourceId === targetSourceId) {
        if (state.currentSourceId === "__ALL_FAVORITES__" && !newFav) {
          if (state.epgData.rawChannels) {
            state.epgData.rawChannels = state.epgData.rawChannels.filter(
              (c) => c.id !== ch.id,
            );
          }
          state.epgData.channels = state.epgData.channels.filter((c) => c.id !== ch.id);
        } else {
          const liveCh = state.epgData.channels.find((c) => c.id === ch.id);
          if (liveCh) liveCh.isFavorite = newFav;
          if (state.epgData.rawChannels) {
            const rawCh = state.epgData.rawChannels.find((c) => c.id === ch.id);
            if (rawCh) rawCh.isFavorite = newFav;
          }
        }
        applyActiveSortToEpgChannels();
        renderEPG();
      }
      renderChannelOrderList();
    };

    // Rejtés kapcsoló
    item.querySelector(".btn-channel-hide").onclick = async (e) => {
      e.stopPropagation();
      const newHidden = await invoke("toggle_channel_hidden", {
        sourceId: actualSourceId,
        channelId: actualChannelId,
      });
      ch.isHidden = newHidden;
      if (targetSourceId === state.currentSourceId) {
        const liveCh = state.epgData.channels.find((c) => c.id === ch.id);
        if (liveCh) liveCh.isHidden = newHidden;
        if (state.epgData.rawChannels) {
          const rawCh = state.epgData.rawChannels.find((c) => c.id === ch.id);
          if (rawCh) rawCh.isHidden = newHidden;
        }
        applyActiveSortToEpgChannels();
        renderEPG();
      }
      renderChannelOrderList();
    };

    // Együttes mentés (Név és Logó egyszerre)
    const logoInput = item.querySelector(".channel-logo-input");
    const renameInput = item.querySelector(".channel-rename-input");
    const logoSearchBtn = item.querySelector(".btn-channel-logo-search");

    if (logoSearchBtn) {
      logoSearchBtn.onclick = async (e) => {
        e.stopPropagation();
        const queryName =
          renameInput.value.trim() ||
          ch.customName ||
          ch.originalName ||
          ch.name;
        const srcObj = state.sources.find((s) => s.id === actualSourceId);
        const country =
          ch.countryCode || (srcObj ? srcObj.countryCode : null) || "HU";
        const selectedUrl = await openLogoSearchModal(queryName, country);
        if (selectedUrl) {
          logoInput.value = selectedUrl;
          saveBoth();
        }
      };
    }

    const saveBoth = async () => {
      const newName = renameInput.value.trim();
      const newLogo = logoInput.value.trim();

      await invoke("rename_channel", {
        sourceId: actualSourceId,
        channelId: actualChannelId,
        name: newName,
      });
      await invoke("set_channel_logo", {
        sourceId: actualSourceId,
        channelId: actualChannelId,
        logoUrl: newLogo,
      });

      ch.customName = newName || null;
      ch.customIcon = newLogo || null;

      if (targetSourceId === state.currentSourceId) {
        const liveCh = state.epgData.channels.find((c) => c.id === ch.id);
        if (liveCh) {
          liveCh.customName = ch.customName;
          liveCh.name = ch.customName || liveCh.originalName || liveCh.name;
          liveCh.customIcon = ch.customIcon;
          liveCh.cachedIcon = null;
          if (newLogo) {
            invoke("cache_logo", { url: newLogo })
              .then((dataUri) => {
                liveCh.cachedIcon = dataUri;
                renderEPG();
              })
              .catch(() => {});
          }
        }
        if (state.epgData.rawChannels) {
          const rawCh = state.epgData.rawChannels.find((c) => c.id === ch.id);
          if (rawCh) {
            rawCh.customName = ch.customName;
            rawCh.name = ch.customName || rawCh.originalName || rawCh.name;
            rawCh.customIcon = ch.customIcon;
            rawCh.cachedIcon = null;
          }
        }
        applyActiveSortToEpgChannels();
        renderEPG();
      }

      // Frissítsük közvetlenül a DOM elemet újrarajzolás nélkül, így megmarad a fókusz és a görgetési pozíció
      const nameSpan = item.querySelector(".channel-order-name");
      if (nameSpan) {
        nameSpan.textContent = ch.customName || ch.originalName || ch.name;
        nameSpan.title = ch.customName || ch.originalName || ch.name;
      }
      const logoPreview = item.querySelector(".channel-order-logo-preview");
      const effectiveLogo = ch.customIcon || ch.icon || "";
      if (logoPreview) {
        const tvPlaceholderSvg = Icons
          ? Icons.get("device-tv", { size: 18 })
          : "TV";
        logoPreview.innerHTML = "";
        if (effectiveLogo) {
          appendSafeImage(logoPreview, effectiveLogo);
        } else {
          logoPreview.innerHTML = `<span class="logo-preview-placeholder">${tvPlaceholderSvg}</span>`;
        }
      }
      const actionsWrap = item.querySelector(".channel-edit-fields-actions");
      if (actionsWrap) {
        actionsWrap.innerHTML = `
                <button class="btn-logo-action btn-channel-data-save btn btn-green">${t("btn_rename_save")}</button>
                ${ch.customName || ch.customIcon ? `<button class="btn-logo-action btn-channel-data-reset btn btn-red btn-icon-only" title="${t("btn_rename_reset")}">${iconHtml("x", 13)}</button>` : ""}
              `;
        const newSaveBtn = actionsWrap.querySelector(".btn-channel-data-save");
        if (newSaveBtn)
          newSaveBtn.onclick = (e) => {
            e.stopPropagation();
            saveBoth();
          };
        const newResetBtn = actionsWrap.querySelector(
          ".btn-channel-data-reset",
        );
        if (newResetBtn)
          newResetBtn.onclick = (e) => {
            e.stopPropagation();
            resetBoth();
          };
      }
    };

    const resetBoth = async () => {
      await invoke("rename_channel", {
        sourceId: actualSourceId,
        channelId: actualChannelId,
        name: "",
      });
      await invoke("set_channel_logo", {
        sourceId: actualSourceId,
        channelId: actualChannelId,
        logoUrl: "",
      });

      ch.customName = null;
      ch.customIcon = null;

      if (targetSourceId === state.currentSourceId) {
        const liveCh = state.epgData.channels.find((c) => c.id === ch.id);
        if (liveCh) {
          liveCh.customName = null;
          liveCh.name = liveCh.originalName || liveCh.name;
          liveCh.customIcon = null;
          liveCh.cachedIcon = null;
        }
        if (state.epgData.rawChannels) {
          const rawCh = state.epgData.rawChannels.find((c) => c.id === ch.id);
          if (rawCh) {
            rawCh.customName = null;
            rawCh.name = rawCh.originalName || rawCh.name;
            rawCh.customIcon = null;
            rawCh.cachedIcon = null;
          }
        }
        applyActiveSortToEpgChannels();
        renderEPG();
      }

      renameInput.value = "";
      logoInput.value = "";
      const nameSpan = item.querySelector(".channel-order-name");
      if (nameSpan) {
        nameSpan.textContent = ch.originalName || ch.name;
        nameSpan.title = ch.originalName || ch.name;
      }
      const logoPreview = item.querySelector(".channel-order-logo-preview");
      const effectiveLogo = ch.icon || "";
      if (logoPreview) {
        const tvPlaceholderSvg = Icons
          ? Icons.get("device-tv", { size: 18 })
          : "TV";
        logoPreview.innerHTML = "";
        if (effectiveLogo) {
          appendSafeImage(logoPreview, effectiveLogo);
        } else {
          logoPreview.innerHTML = `<span class="logo-preview-placeholder">${tvPlaceholderSvg}</span>`;
        }
      }
      const actionsWrap = item.querySelector(".channel-edit-fields-actions");
      if (actionsWrap) {
        actionsWrap.innerHTML = `
                <button class="btn-logo-action btn-channel-data-save btn btn-green">${t("btn_rename_save")}</button>
              `;
        const newSaveBtn = actionsWrap.querySelector(".btn-channel-data-save");
        if (newSaveBtn)
          newSaveBtn.onclick = (e) => {
            e.stopPropagation();
            saveBoth();
          };
      }
    };

    const saveBtn = item.querySelector(".btn-channel-data-save");
    if (saveBtn)
      saveBtn.onclick = (e) => {
        e.stopPropagation();
        saveBoth();
      };

    const resetBtn = item.querySelector(".btn-channel-data-reset");
    if (resetBtn)
      resetBtn.onclick = (e) => {
        e.stopPropagation();
        resetBoth();
      };

    renameInput.onkeydown = (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        saveBoth();
      }
    };

    logoInput.onkeydown = (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        saveBoth();
      }
    };

    // Nyilakkal mozgatás
    item.querySelector(".btn-up").onclick = (e) => {
      e.stopPropagation();
      const prev = item.previousElementSibling;
      if (prev) {
        container.insertBefore(item, prev);
        updateAndSave();
      }
    };

    item.querySelector(".btn-down").onclick = (e) => {
      e.stopPropagation();
      const next = item.nextElementSibling;
      if (next) {
        container.insertBefore(next, item);
        updateAndSave();
      }
    };

    const handle = item.querySelector(".drag-handle");
    setupChannelOrderPointerDrag(item, handle, container, updateAndSave);

    container.appendChild(item);
  });

  if (savedContainerScroll > 0) {
    container.scrollTop = savedContainerScroll;
  }
  if (savedContentAreaScroll > 0 && contentArea) {
    contentArea.scrollTop = savedContentAreaScroll;
  }
}

export function initChannels() {
  initChannelSortHandlers();
  initChannelFilterHandlers();

  document.getElementById("resetChannelOrderBtn").onclick = async () => {
    const select = document.getElementById("channelOrderSourceSelect");
    const targetSourceId = select.value || state.currentSourceId;
    if (!targetSourceId) return;

    await invoke("save_channel_order", {
      sourceId: targetSourceId,
      order: [],
    });
    if (targetSourceId === state.currentSourceId) {
      await fetchEPG();
    }
    await renderChannelOrderList();
  };
}
