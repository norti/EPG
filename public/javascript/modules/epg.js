function parseEPGDate(str, manualOffsetHours = 0) {
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

function formatDateLabel(dateStr) {
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
  if (currentLang === "english") {
    return `${dayName} - ${months[date.getMonth()]} ${dayNum}, ${date.getFullYear()}`;
  }
  return `${dayName} - ${date.getFullYear()}. ${months[date.getMonth()]} ${dayNum}.`;
}

async function updateHeaderSortDropdown() {
  // Header sort selector removed; channel sorting is managed in Settings -> Channels.
}

function applyActiveSortToEpgChannels() {
  if (!epgData || !epgData.channels || epgData.channels.length === 0) return;
  if (!epgData.rawChannels) {
    epgData.rawChannels = [...epgData.channels];
  }
  epgData.channels = [...epgData.rawChannels];
}

async function fetchEPG(options = {}) {
  if (!currentSourceId) {
    renderEmptySourcesState();
    return;
  }

  updateAllFavBtnState();
  document.getElementById("status").textContent = t("status_loading");
  try {
    epgData = await invoke("get_epg_data", { sourceId: currentSourceId });

    if (currentSourceId === "__ALL_FAVORITES__") {
      if (!epgData.channels || epgData.channels.length === 0) {
        document.getElementById("status").textContent = t("status_ready");
        await updateHeaderSortDropdown();
        renderEmptyFavoritesState();
        return;
      }
    } else if (!epgData.channels || epgData.channels.length === 0) {
      await downloadSource(currentSourceId);
      epgData = await invoke("get_epg_data", {
        sourceId: currentSourceId,
      });
    }

    epgData.rawChannels = [...(epgData.channels || [])];
    await updateHeaderSortDropdown();
    applyActiveSortToEpgChannels();

    const currentSrcObj = sources.find((s) => s.id === currentSourceId);
    const defaultOffset = currentSrcObj ? currentSrcObj.timeOffset || 0 : 0;

    epgData.programmes.forEach((p) => {
      const offset =
        currentSourceId === "__ALL_FAVORITES__" &&
        p.timeOffset !== undefined &&
        p.timeOffset !== null
          ? p.timeOffset
          : defaultOffset;
      p.start = parseEPGDate(p.start, offset);
      p.stop = parseEPGDate(p.stop, offset);
    });

    if (window.TraktUI) {
      await window.TraktUI.prepareMatchesForEpg(epgData.programmes);
    }

    document.getElementById("status").textContent = t("status_ready");
    setupDateSelector(options.targetDateStr || null);
    renderEPG();
    if (!options.skipScrollToNow) {
      scrollToNow();
    }

    // Cache logos in the background for channels that have a URL but no cached data (throttled queue)
    const uncachedChannels = epgData.channels.filter((c) => {
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
          const batch = uncachedChannels.slice(i, i + batchSize);
          await Promise.all(
            batch.map(async (channel) => {
              const logoUrl = channel.customIcon || channel.icon;
              try {
                const dataUri = await invoke("cache_logo", { url: logoUrl });
                channel.cachedIcon = dataUri;
              } catch (_) {}
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

function navigateDate(delta) {
  const dateSelect = document.getElementById("dateSelect");
  if (!dateSelect || dateSelect.options.length === 0) return;
  const newIdx = dateSelect.selectedIndex + delta;
  if (newIdx >= 0 && newIdx < dateSelect.options.length) {
    dateSelect.selectedIndex = newIdx;
    selectedDateStr = dateSelect.value;
    updateDateNavBtns();
    renderEPG();
  }
}

function setupDateSelector(preferredDateStr = null) {
  const dateSelect = document.getElementById("dateSelect");
  if (!dateSelect) return;
  const dates = new Set();

  epgData.programmes.forEach((p) => {
    if (p.start && !isNaN(p.start.getTime())) {
      const y = p.start.getFullYear();
      const m = String(p.start.getMonth() + 1).padStart(2, "0");
      const d = String(p.start.getDate()).padStart(2, "0");
      dates.add(`${y}-${m}-${d}`);
    }
  });

  const sortedDates = Array.from(dates).sort();
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
    selectedDateStr = preferredDateStr;
  } else if (sortedDates.includes(todayStr)) {
    selectedDateStr = todayStr;
  } else if (sortedDates.length > 0) {
    selectedDateStr = sortedDates[0];
  }
  dateSelect.value = selectedDateStr;

  updateDateNavBtns();

  dateSelect.onchange = (e) => {
    selectedDateStr = e.target.value;
    updateDateNavBtns();
    renderEPG();
  };
}

function setupMainHeaderPointerDrag(headerItem, columnWrapper, channelId) {
  const isHorizontal = currentLayout === "horizontal";
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
        } catch (err) {}
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
      } catch (err) {}

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
        const fromIdx = epgData.channels.findIndex((c) => c.id === channelId);
        const toIdx = epgData.channels.findIndex((c) => c.id === targetId);

        if (fromIdx !== -1 && toIdx !== -1) {
          const [moved] = epgData.channels.splice(fromIdx, 1);
          epgData.channels.splice(toIdx, 0, moved);
          const newOrder = epgData.channels.map((c) => c.id);
          await invoke("save_channel_order", {
            sourceId: currentSourceId,
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

  const isHorizontal = currentLayout === "horizontal";
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

  const { prog, isFav, isTrakt, startStr, stopStr } = data;
  el.innerHTML = `
          <div class="prog-header">
              <span class="start-badge">${startStr}</span>
              <span class="prog-title" title="${escapeHtml(prog.title)}">${escapeHtml(prog.title)}</span>
              <span class="fav-star ${isFav ? "active" : ""}">${iconHtml(isFav ? "star-filled" : "star", 15)}</span>
          </div>
          ${prog.desc ? `<div class="prog-desc">${escapeHtml(prog.desc)}</div>` : ""}
          ${isTrakt ? `<span class="trakt-epg-badge" data-icon="trakt-epg-badge" data-icon-size="12" title="${escapeHtml(t("trakt_on_watchlist_tooltip"))}"></span>` : ""}
        `;

  const star = el.querySelector(".fav-star");
  if (star) {
    star.onclick = (e) => {
      e.stopPropagation();
      toggleFavorite(prog.title);
    };
  }

  if (isTrakt && window.Icons) {
    const badge = el.querySelector(".trakt-epg-badge");
    if (badge) window.Icons.replace(el);
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

  const dates = new Set();
  epgData.programmes.forEach((p) => {
    if (p.channel === channel.id && p.start && !isNaN(p.start.getTime())) {
      const y = p.start.getFullYear();
      const m = String(p.start.getMonth() + 1).padStart(2, "0");
      const d = String(p.start.getDate()).padStart(2, "0");
      dates.add(`${y}-${m}-${d}`);
    }
  });

  const sortedDates = Array.from(dates).sort();
  const totalMinutes = 1440;
  const now = new Date();
  let firstMatchEl = null;

  const progQuery =
    filterText && window.SearchUtils
      ? window.SearchUtils.prepareQuery(filterText)
      : null;
  const channelQuery =
    filterText && window.SearchUtils
      ? window.SearchUtils.prepareQuery(filterText, { minFuzzyLength: 6 })
      : null;
  const chNameMatch = filterText
    ? channelQuery
      ? window.SearchUtils.matchPrepared(channel.name, channelQuery)
      : fuzzySearchMatch(channel.name, filterText, { minFuzzyLength: 6 })
    : false;
  const progMatchCache = filterText ? new Map() : null;
  const isProgMatch = (title) => {
    if (!filterText) return false;
    const key = title || "";
    let hit = progMatchCache.get(key);
    if (hit === undefined) {
      hit = progQuery
        ? window.SearchUtils.matchPrepared(key, progQuery)
        : fuzzySearchMatch(key, filterText);
      progMatchCache.set(key, hit);
    }
    return hit;
  };

  sortedDates.forEach((dateStr) => {
    const dayStart = new Date(`${dateStr}T00:00:00`);
    const dayEndDate = new Date(dayStart);
    dayEndDate.setDate(dayEndDate.getDate() + 1);
    const dayEnd = dayEndDate;

    const dayProgs = epgData.programmes.filter(
      (p) => p.channel === channel.id && p.stop > dayStart && p.start < dayEnd,
    );

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

      const isFav = favorites.includes(prog.title);
      const isTrakt = window.TraktUI?.isOnWatchlist(prog.title) ?? false;
      const isHighlight = filterText ? isProgMatch(prog.title) : false;
      const isActual = prog.start <= now && prog.stop > now;

      const progEl = document.createElement("div");
      progEl.className = `programme ${isFav ? "favorite" : ""} ${isTrakt ? "trakt-watchlist" : ""} ${isHighlight ? "highlight" : ""} ${isActual ? "actual" : ""}`;
      if (isHorizontal) {
        progEl.style.left = `${startDiffMin * minuteScale}px`;
        progEl.style.width = `${Math.max(20, durationMin * minuteScale - 1)}px`;
      } else {
        progEl.style.top = `${startDiffMin * currentMinuteHeight}px`;
        progEl.style.height = `${Math.max(16, durationMin * currentMinuteHeight - 1)}px`;
      }

      const startStr = prog.start.toTimeString().substring(0, 5);
      const stopStr = prog.stop.toTimeString().substring(0, 5);

      progEl._progData = {
        prog,
        isFav,
        isTrakt,
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

      progEl.onclick = () =>
        openModal(prog.title, `${startStr} - ${stopStr}`, prog.desc, channel);

      grid.appendChild(progEl);
      if (programObserver) {
        programObserver.observe(progEl);
      }
    });

    wrapper.appendChild(grid);
    channelsContainer.appendChild(wrapper);
  });

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

function renderEPG() {
  if (!currentSourceId || sources.length === 0) {
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

  const isHorizontal = currentLayout === "horizontal";
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
    `${currentMinuteHeight}px`,
  );
  document.documentElement.style.setProperty(
    "--minute-scale",
    `${minuteScale}px`,
  );
  document.documentElement.style.setProperty("--step-min", currentStepMin);

  const totalMinutes = 1440;
  for (let min = 0; min < totalMinutes; min += currentStepMin) {
    const h = Math.floor(min / 60) % 24;
    const m = min % 60;
    const timeDiv = document.createElement("div");
    timeDiv.className = isHorizontal ? "timeline-slot" : "time-slot";
    if (m === 0) timeDiv.classList.add("full-hour");
    timeDiv.textContent = `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
    timeColumn.appendChild(timeDiv);
  }

  const dateNavGroup = document.querySelector(".date-nav-group");
  if (activeSingleChannelId) {
    if (dateNavGroup) dateNavGroup.style.display = "none";
  } else {
    if (dateNavGroup) dateNavGroup.style.display = "";
  }
  updateGridCornerButton();

  const activeBadge = document.getElementById("activeChannelBadge");
  const activeLogo = document.getElementById("activeChannelLogo");
  const activeName = document.getElementById("activeChannelName");

  if (activeBadge) {
    if (activeSingleChannelId) {
      const channel = epgData.channels.find(
        (c) => c.id === activeSingleChannelId,
      );
      if (channel) {
        const effectiveLogo =
          channel.cachedIcon || channel.customIcon || channel.icon || "";
        if (activeLogo) {
          if (effectiveLogo) {
            activeLogo.src = effectiveLogo;
            activeLogo.style.display = "inline-block";
          } else {
            activeLogo.style.display = "none";
          }
        }
        if (activeName) {
          activeName.textContent = channel.name;
        }
        activeBadge.style.display = "inline-flex";
        activeBadge.title = t("grid_back_btn_title");
        activeBadge.onclick = () => {
          activeSingleChannelId = null;
          renderEPG();
          scrollToNow();
        };
      } else {
        activeBadge.style.display = "none";
      }
    } else {
      activeBadge.style.display = "none";
    }
  }

  if (activeSingleChannelId) {
    const channel = epgData.channels.find(
      (c) => c.id === activeSingleChannelId,
    );
    if (channel) {
      renderSingleChannelMultiDay(channel, filterText, isHorizontal, minuteScale);
      return;
    }
    activeSingleChannelId = null;
    const activeBadge = document.getElementById("activeChannelBadge");
    if (activeBadge) activeBadge.style.display = "none";
  }

  const dayStart = new Date(`${selectedDateStr}T00:00:00`);
  // Use next day's midnight in wall-clock time (DST-safe)
  const dayEndDate = new Date(dayStart);
  dayEndDate.setDate(dayEndDate.getDate() + 1);
  const dayEnd = dayEndDate;
  const now = new Date();

  let firstMatchEl = null;

  const progQuery =
    filterText && window.SearchUtils
      ? window.SearchUtils.prepareQuery(filterText)
      : null;
  const channelQuery =
    filterText && window.SearchUtils
      ? window.SearchUtils.prepareQuery(filterText, { minFuzzyLength: 6 })
      : null;
  const progMatchCache = filterText ? new Map() : null;
  const isProgMatch = (title) => {
    if (!filterText) return false;
    const key = title || "";
    let hit = progMatchCache.get(key);
    if (hit === undefined) {
      hit = progQuery
        ? window.SearchUtils.matchPrepared(key, progQuery)
        : fuzzySearchMatch(key, filterText);
      progMatchCache.set(key, hit);
    }
    return hit;
  };

  epgData.channels.forEach((channel) => {
    if (channel.isHidden) return;

    const channelProgs = epgData.programmes.filter(
      (p) => p.channel === channel.id && p.stop > dayStart && p.start < dayEnd,
    );

    if (channelProgs.length === 0) return;

    if (filterText) {
      const chNameMatch = channelQuery
        ? window.SearchUtils.matchPrepared(channel.name, channelQuery)
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

    // Use cached base64 icon if available, else fall back to URL
    const effectiveLogo =
      channel.cachedIcon || channel.customIcon || channel.icon || "";

    const showSourceBadge =
      currentSourceId === "__ALL_FAVORITES__" || !!channel.sourceName;
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
        activeSingleChannelId = channel.id;
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

        const targetSrc = channel.sourceId || currentSourceId;
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
          if (result.logo) {
            invoke("cache_logo", { url: result.logo })
              .then((dataUri) => {
                channel.cachedIcon = dataUri;
                renderEPG();
              })
              .catch(() => {});
          }
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

    const favBtn = headerItem.querySelector(".channel-header-fav-btn");
    if (favBtn) {
      favBtn.onclick = async (e) => {
        e.stopPropagation();
        const targetSrc = channel.sourceId || currentSourceId;
        const targetChId = channel.id.includes("::")
          ? channel.id.split("::").slice(1).join("::")
          : channel.id;
        const newFav = await invoke("toggle_channel_favorite", {
          sourceId: targetSrc,
          channelId: targetChId,
        });
        channel.isFavorite = newFav;
        if (currentSourceId === "__ALL_FAVORITES__" && !newFav) {
          epgData.channels = epgData.channels.filter(
            (c) => c.id !== channel.id,
          );
          if (epgData.channels.length === 0) {
            renderEmptyFavoritesState();
            return;
          }
        } else {
          epgData.channels.sort((a, b) => {
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

      const isFav = favorites.includes(prog.title);
      const isTrakt = window.TraktUI?.isOnWatchlist(prog.title) ?? false;
      const isHighlight = filterText ? isProgMatch(prog.title) : false;
      const isActual = prog.start <= now && prog.stop > now;

      const progEl = document.createElement("div");
      progEl.className = `programme ${isFav ? "favorite" : ""} ${isTrakt ? "trakt-watchlist" : ""} ${isHighlight ? "highlight" : ""} ${isActual ? "actual" : ""}`;
      if (isHorizontal) {
        progEl.style.left = `${startDiffMin * minuteScale}px`;
        progEl.style.width = `${Math.max(20, durationMin * minuteScale - 1)}px`;
      } else {
        progEl.style.top = `${startDiffMin * currentMinuteHeight}px`;
        progEl.style.height = `${Math.max(16, durationMin * currentMinuteHeight - 1)}px`;
      }

      const startStr = prog.start.toTimeString().substring(0, 5);
      const stopStr = prog.stop.toTimeString().substring(0, 5);

      progEl._progData = {
        prog,
        isFav,
        isTrakt,
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

      progEl.onclick = () =>
        openModal(prog.title, `${startStr} - ${stopStr}`, prog.desc, channel);

      grid.appendChild(progEl);
      if (programObserver) {
        programObserver.observe(progEl);
      }
    });

    wrapper.appendChild(grid);
    channelsContainer.appendChild(wrapper);
  });

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

async function jumpToEpgAiring(airing) {
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
    jumpOverlay.style.display = "flex";
  }

  // Yield a frame so the progress layer paints immediately before heavy DOM/IPC work
  await new Promise((resolve) => requestAnimationFrame(resolve));

  try {
    const startDate = parseEPGDate(airing.start, airing.timeOffset || 0);
    const targetDateStr = `${startDate.getFullYear()}-${String(startDate.getMonth() + 1).padStart(2, "0")}-${String(startDate.getDate()).padStart(2, "0")}`;
    const startMin = startDate.getHours() * 60 + startDate.getMinutes();

    hidePosterZoom();
    closeFixOverlay();
    if (window.TraktUI) {
      window.TraktUI.detachFromModal();
    }
    document.getElementById("modalOverlay").classList.remove("active");

    if (window.TraktUI && window.TraktUI.isTraktMode()) {
      window.TraktUI.exitTraktMode({ skipRender: true });
    }

    activeSingleChannelId = null;

    const searchInput = document.getElementById("searchInput");
    if (searchInput && searchInput.value.trim()) {
      const q = searchInput.value.trim();
      if (
        !fuzzySearchMatch(airing.title || "", q) &&
        !fuzzySearchMatch(airing.channelName || "", q, { minFuzzyLength: 6 })
      ) {
        searchInput.value = "";
      }
    }

    if (currentSourceId !== airing.sourceId) {
      currentSourceId = airing.sourceId;
      const sourceSelect = document.getElementById("sourceSelect");
      if (sourceSelect) sourceSelect.value = currentSourceId;
      updateAllFavBtnState();
      invoke("set_setting", {
        key: "current_source_id",
        value: currentSourceId,
      }).catch(() => {});
      await fetchEPG({ skipScrollToNow: true, targetDateStr });
    } else {
      const dateSelect = document.getElementById("dateSelect");
      if (dateSelect) {
        const hasDate = Array.from(dateSelect.options).some(
          (o) => o.value === targetDateStr,
        );
        if (hasDate) {
          selectedDateStr = targetDateStr;
          dateSelect.value = targetDateStr;
          updateDateNavBtns();
        }
      } else {
        selectedDateStr = targetDateStr;
      }
      renderEPG();
    }

    const scrollToTarget = () => {
      const viewport = document.getElementById("epgViewport");
      if (!viewport) return;

      const wrappers = Array.from(
        document.querySelectorAll(
          currentLayout === "horizontal"
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

      if (currentLayout === "horizontal") {
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
          : startMin * currentMinuteHeight;
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
      jumpOverlay.style.display = "none";
    }
  }
}

window.EpgApp = {
  isFavoriteTitle: (title) => favorites.includes(title),
  toggleFavorite,
  jumpToEpgAiring,
};

function renderNowLine() {
  const oldLines = document.querySelectorAll(".now-line, .now-line-vertical");
  oldLines.forEach((l) => l.remove());

  const now = new Date();
  const todayStr = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;

  if (activeSingleChannelId) {
    const todayWrapper = document.querySelector(
      `[data-date-str="${todayStr}"]`,
    );
    if (!todayWrapper) return;

    const currentMin = now.getHours() * 60 + now.getMinutes();

    if (currentLayout === "horizontal") {
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
      const topPx = currentMin * currentMinuteHeight;
      const line = document.createElement("div");
      line.className = "now-line";
      line.style.top = `${topPx}px`;
      progGrid.appendChild(line);
    }
    return;
  }

  if (selectedDateStr === todayStr) {
    const currentMin = now.getHours() * 60 + now.getMinutes();

    if (currentLayout === "horizontal") {
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
      const topPx = currentMin * currentMinuteHeight + headerHeight;
      const line = document.createElement("div");
      line.className = "now-line";
      line.style.top = `${topPx}px`;
      document.getElementById("channelsContainer").appendChild(line);
    }
  }
}

function scrollToNow() {
  const viewport = document.getElementById("epgViewport");
  if (!viewport) return;

  const now = new Date();
  const todayStr = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;

  if (activeSingleChannelId) {
    const todayWrapper = document.querySelector(
      `[data-date-str="${todayStr}"]`,
    );
    const currentMin = now.getHours() * 60 + now.getMinutes();

    if (currentLayout === "horizontal") {
      const minuteScale = getEffectiveMinuteScale();
      viewport.scrollLeft = Math.max(0, currentMin * minuteScale - 300);
      if (todayWrapper) {
        viewport.scrollTop = Math.max(0, todayWrapper.offsetTop - 50);
      }
    } else {
      viewport.scrollTop = Math.max(0, currentMin * currentMinuteHeight - 200);
      if (todayWrapper) {
        viewport.scrollLeft = Math.max(0, todayWrapper.offsetLeft - 80);
      }
    }
    return;
  }

  if (currentLayout === "horizontal") {
    // Find the earliest starting time among all currently running programs
    let earliestStartMin = null;
    if (selectedDateStr === todayStr && epgData.programmes) {
      epgData.programmes.forEach((p) => {
        if (p.start <= now && p.stop > now) {
          const sMin = p.start.getHours() * 60 + p.start.getMinutes();
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

const mainSearchInput = document.getElementById("searchInput");
mainSearchInput.oninput = () => {
  clearTimeout(searchTimeout);
  searchTimeout = setTimeout(renderEPG, 300);
};
mainSearchInput.onkeydown = (e) => {
  if (e.key === "Enter") {
    clearTimeout(searchTimeout);
    renderEPG();
  }
};

document.getElementById("searchBtn").onclick = () => {
  clearTimeout(searchTimeout);
  renderEPG();
};
document.getElementById("clearSearchBtn").onclick = () => {
  clearTimeout(searchTimeout);
  mainSearchInput.value = "";
  renderEPG();
};

function getEffectiveMinuteScale() {
  if (currentLayout === "horizontal") {
    return HORIZONTAL_SCALE_MAP[currentStepMin] || currentMinuteHeight * 2;
  }
  return currentMinuteHeight;
}

const viewSelect = document.getElementById("viewSelect");
if (viewSelect) {
  viewSelect.onchange = async (e) => {
    currentStepMin = parseInt(e.target.value, 10);
    currentMinuteHeight = VIEW_HEIGHT_MAP[currentStepMin] || 2;
    await invoke("set_setting", {
      key: "view_step_min",
      value: String(currentStepMin),
    });
    await invoke("set_setting", {
      key: "view_minute_height",
      value: String(currentMinuteHeight),
    });
    renderEPG();
  };
}

let epgScrollRaf = null;
function scheduleEPGScrollUpdate() {
  if (epgScrollRaf) return;
  epgScrollRaf = requestAnimationFrame(() => {
    epgScrollRaf = null;
    renderNowLine();
  });
}

const layoutToggleBtn = document.getElementById("layoutToggleBtn");
if (layoutToggleBtn) {
  layoutToggleBtn.onclick = async () => {
    currentLayout = currentLayout === "horizontal" ? "vertical" : "horizontal";
    await invoke("set_setting", {
      key: "view_layout",
      value: currentLayout,
    });
    updateLayoutButtonUi();
    renderEPG();
  };
}

const epgViewportEl = document.getElementById("epgViewport");
if (epgViewportEl) {
  epgViewportEl.addEventListener("scroll", scheduleEPGScrollUpdate, {
    passive: true,
  });
}

// --- EPG Viewport drag-to-pan (minden irány, kivéve csatorna header) ---
(function setupViewportPan() {
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
      } catch (err) {}
      viewport.style.cursor = "grabbing";
      viewport.style.userSelect = "none";
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
      } catch (err) {}
      viewport.style.cursor = "";
      viewport.style.userSelect = "";
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
})();
// --- EPG Viewport drag-to-pan vége ---

const gridHomeBtn = document.getElementById("gridHomeBtn");
if (gridHomeBtn) {
  gridHomeBtn.onclick = (e) => {
    e.stopPropagation();
    if (activeSingleChannelId) {
      activeSingleChannelId = null;
      renderEPG();
      scrollToNow();
    } else {
      const viewport = document.getElementById("epgViewport");
      if (viewport) {
        if (currentLayout === "horizontal") {
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
  if (!activeSingleChannelId) {
    if (selectedDateStr !== todayStr) {
      selectedDateStr = todayStr;
      document.getElementById("dateSelect").value = selectedDateStr;
      renderEPG();
    }
  }
  scrollToNow();
};

setInterval(() => {
  renderNowLine();
}, 60000);
