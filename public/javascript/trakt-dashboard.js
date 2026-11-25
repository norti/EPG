/**
 * Trakt.tv Dashboard, Watchlist, Calendars & Progress Drawer renderer
 */
(function () {
  'use strict';

  const { invoke } = window.__TAURI__.core;

  // Concurrency-limited image loader (max 3 parallel cache_logo / TMDB poster calls)
  const imageQueue = [];
  let activeImageLoads = 0;
  const MAX_CONCURRENT_IMAGES = 3;
  const memoryImageCache = new Map();
  const memoryTmdbPosterCache = new Map();
  const pendingTmdbPosterPromises = new Map();
  let imageObserver = null;
  let furtherWeekIndex = 0;

  function resolveTmdbPosterCached(tmdbId, mediaType, size) {
    const key = `${mediaType}:${tmdbId}:${size}`;
    if (memoryTmdbPosterCache.has(key)) {
      return Promise.resolve(memoryTmdbPosterCache.get(key));
    }
    if (pendingTmdbPosterPromises.has(key)) {
      return pendingTmdbPosterPromises.get(key);
    }
    const p = invoke('trakt_resolve_tmdb_poster', {
      tmdbId: Number(tmdbId),
      mediaType,
      size,
    })
      .then((res) => {
        const dataUri = res && res.dataUri ? res.dataUri : null;
        if (dataUri) {
          memoryTmdbPosterCache.set(key, dataUri);
        }
        return dataUri;
      })
      .catch(() => null)
      .finally(() => {
        pendingTmdbPosterPromises.delete(key);
      });
    pendingTmdbPosterPromises.set(key, p);
    return p;
  }

  function enqueueImageLoad(task) {
    if (!task) return;
    if (task.url && memoryImageCache.has(task.url)) {
      task.imgEl.src = memoryImageCache.get(task.url);
      task.imgEl.classList.remove('u-hidden');
      if (task.placeholderEl) task.placeholderEl.classList.add('u-hidden');
      return;
    }
    if (!task.url && task.tmdbId) {
      const key = `${task.tmdbMediaType}:${task.tmdbId}:${task.size}`;
      if (memoryTmdbPosterCache.has(key)) {
        task.imgEl.src = memoryTmdbPosterCache.get(key);
        task.imgEl.classList.remove('u-hidden');
        if (task.placeholderEl) task.placeholderEl.classList.add('u-hidden');
        return;
      }
    }
    imageQueue.push(task);
    pumpImageQueue();
  }

  function pumpImageQueue() {
    while (activeImageLoads < MAX_CONCURRENT_IMAGES && imageQueue.length > 0) {
      const task = imageQueue.shift();
      if (!task || !task.imgEl.isConnected) continue;
      activeImageLoads++;
      (async () => {
        try {
          let dataUri = null;
          if (task.url) {
            dataUri = memoryImageCache.get(task.url);
            if (!dataUri) {
              dataUri = await invoke('get_cached_logo', { url: task.url });
            }
            if (!dataUri) {
              dataUri = await invoke('cache_logo', { url: task.url });
            }
            if (dataUri) {
              memoryImageCache.set(task.url, dataUri);
            }
          } else if (task.tmdbId) {
            dataUri = await resolveTmdbPosterCached(
              task.tmdbId,
              task.tmdbMediaType || 'tv',
              task.size || 'w185'
            );
          }

          if (dataUri && task.imgEl.isConnected) {
            task.imgEl.src = dataUri;
            task.imgEl.classList.remove('u-hidden');
            if (task.placeholderEl) task.placeholderEl.classList.add('u-hidden');
          }
        } catch (_) {
          // Keep placeholder on failure
        } finally {
          activeImageLoads--;
          pumpImageQueue();
        }
      })();
    }
  }

  function ensureImageObserver(rootEl) {
    if (imageObserver) {
      imageObserver.disconnect();
    }
    imageObserver = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          const thumb = entry.target;
          imageObserver.unobserve(thumb);
          const url = thumb.dataset.imgUrl || '';
          const tmdbId = thumb.dataset.tmdbId || '';
          const tmdbMediaType = thumb.dataset.tmdbMediaType || 'tv';
          const size = thumb.dataset.tmdbSize || 'w185';
          const imgEl = thumb.querySelector('img');
          const phEl = thumb.querySelector('.trakt-thumb-placeholder');
          if ((url || tmdbId) && imgEl) {
            enqueueImageLoad({ url, tmdbId, tmdbMediaType, size, imgEl, placeholderEl: phEl });
          }
        });
      },
      { root: rootEl || null, rootMargin: '250px', threshold: 0.01 }
    );
  }

  function observeThumb(thumbEl, posterPath, size = 'w342', tmdbId = null, tmdbMediaType = 'tv') {
    if (posterPath) {
      const url = posterPath.startsWith('http')
        ? posterPath
        : `https://image.tmdb.org/t/p/${size}${posterPath}`;
      thumbEl.dataset.imgUrl = url;
      if (memoryImageCache.has(url)) {
        const imgEl = thumbEl.querySelector('img');
        const phEl = thumbEl.querySelector('.trakt-thumb-placeholder');
        if (imgEl) {
          imgEl.src = memoryImageCache.get(url);
          imgEl.classList.remove('u-hidden');
          if (phEl) phEl.classList.add('u-hidden');
        }
        return;
      }
      if (imageObserver) {
        imageObserver.observe(thumbEl);
      }
      return;
    }

    if (tmdbId) {
      thumbEl.dataset.tmdbId = String(tmdbId);
      thumbEl.dataset.tmdbMediaType = tmdbMediaType || 'tv';
      thumbEl.dataset.tmdbSize = size;
      const key = `${thumbEl.dataset.tmdbMediaType}:${tmdbId}:${size}`;
      if (memoryTmdbPosterCache.has(key)) {
        const imgEl = thumbEl.querySelector('img');
        const phEl = thumbEl.querySelector('.trakt-thumb-placeholder');
        if (imgEl) {
          imgEl.src = memoryTmdbPosterCache.get(key);
          imgEl.classList.remove('u-hidden');
          if (phEl) phEl.classList.add('u-hidden');
        }
        return;
      }
      if (imageObserver) {
        imageObserver.observe(thumbEl);
      }
    }
  }

  function pad2(n) {
    return String(n ?? 0).padStart(2, '0');
  }

  function esc(str) {
    if (str === null || str === undefined) return '';
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  function iconHtml(name, size = 16) {
    return window.Icons ? window.Icons.get(name, { size }) : '';
  }

  function createThumbElement(posterPath, size = 'w342', fallbackIcon = 'device-tv', tmdbId = null, tmdbMediaType = 'tv') {
    const thumb = document.createElement('div');
    thumb.className = 'trakt-card-thumb';
    thumb.innerHTML = `
      <img alt="" class="u-hidden" />
      <span class="trakt-thumb-placeholder">${iconHtml(fallbackIcon, 24)}</span>
    `;
    observeThumb(thumb, posterPath, size, tmdbId, tmdbMediaType);
    return thumb;
  }

  function formatEpisodeCode(season, number) {
    return `S${pad2(season)}E${pad2(number)}`;
  }

  function getDisplayShowTitle(item) {
    if (typeof currentLang !== 'undefined' && currentLang === 'hungarian' && item.showTitleHu) {
      return item.showTitleHu;
    }
    return item.showTitle || item.titleHu || item.title || '';
  }

  function renderEpisodeCard(ep, options = {}) {
    const isPosterCard = !!options.posterCard;
    const isContinue = !!(options.isContinueWatching || ep.isContinueWatching);
    const card = document.createElement('div');
    card.className = `${isPosterCard ? 'trakt-poster-card' : 'trakt-card'} ${options.compact ? 'trakt-card-compact' : ''} ${isContinue ? 'trakt-card-continue' : ''} ${ep.watched ? 'trakt-card-watched' : ''}`;

    const thumb = createThumbElement(ep.posterPath, 'w342', 'device-tv', ep.showTmdbId, 'tv');
    if (isPosterCard) {
      thumb.classList.add('trakt-poster-card-thumb');
    }
    if (ep.watched && options.showWatched) {
      const badge = document.createElement('span');
      badge.className = 'trakt-watched-thumb-badge';
      badge.title = t('trakt_status_watched');
      badge.innerHTML = iconHtml('trakt-watched-done', 16);
      thumb.appendChild(badge);
    }
    card.appendChild(thumb);

    const showTitle = getDisplayShowTitle(ep);
    const epCode = formatEpisodeCode(ep.season, ep.number);
    const epName = ep.episodeTitle || '';
    const epTitleSuffix = epName ? ` – ${epName}` : '';
    const nowDate = options.nowDate || new Date();
    const localTime = window.TraktDates ? window.TraktDates.formatLocalTime(ep.firstAired) : '';
    let datePrefix = '';
    if (options.showDate && ep.firstAired) {
      const d = new Date(ep.firstAired);
      const locale = typeof currentLang !== 'undefined' && currentLang === 'english' ? 'en-US' : 'hu-HU';
      datePrefix = d.toLocaleDateString(locale, { weekday: 'short', month: 'short', day: 'numeric' }) + ' · ';
    }
    const continueLabel = isContinue
      ? (ep.aired > 0 ? `${t('trakt_continue_badge')} · ${ep.completed}/${ep.aired}` : t('trakt_continue_badge'))
      : '';
    let rightMetaText = `${datePrefix}${localTime}`;
    if (options.showDynamicPastDay || isContinue) {
      const dynamicMeta = window.TraktDates
        ? window.TraktDates.formatDashboardReleaseMeta(ep.firstAired, nowDate, {
            yesterday: t('day_yesterday'),
          })
        : localTime;
      rightMetaText = dynamicMeta || continueLabel;
    }
    const tooltipParts = [epCode];
    if (rightMetaText) {
      tooltipParts.push(
        isContinue && localTime && rightMetaText !== localTime
          ? `${rightMetaText} ${localTime}`
          : rightMetaText
      );
    }
    if (continueLabel) tooltipParts.push(continueLabel);
    const codeTooltip = tooltipParts.join(' · ');
    const netHtml = ep.network ? `<span class="trakt-card-network">${esc(ep.network)}</span>` : '';

    const info = document.createElement('div');
    if (isPosterCard) {
      info.className = 'trakt-poster-card-body';
      info.innerHTML = `
        <div class="trakt-pcard-show" title="${esc(showTitle)}">${esc(showTitle)}</div>
        <div class="trakt-pcard-code" title="${esc(codeTooltip)}">
          <span>${esc(epCode)}</span>
          <span class="trakt-pcard-time ${isContinue ? 'trakt-continue-pill' : ''}">${esc(rightMetaText)}</span>
        </div>
        <div class="trakt-pcard-ep" title="${esc(epName)}">${esc(epName || '—')}</div>
      `;
    } else {
      info.className = 'trakt-card-info';
      info.innerHTML = `
        <div class="trakt-card-show" title="${esc(showTitle)}">${esc(showTitle)}</div>
        <div class="trakt-card-ep" title="${esc(epCode + epTitleSuffix)}">${esc(epCode + epTitleSuffix)}</div>
        <div class="trakt-card-meta">
          <span class="trakt-card-time">${esc(rightMetaText)}</span>
          ${netHtml}
        </div>
      `;
    }

    const actions = document.createElement('div');
    actions.className = isPosterCard ? 'trakt-pcard-actions' : 'trakt-card-actions';

    if (options.showWatched) {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = `btn trakt-watched-btn ${ep.watched ? 'is-watched' : ''}`;
      if (ep.watched) {
        btn.disabled = true;
        btn.innerHTML = `${iconHtml('trakt-watched-done', 14)} <span>${esc(t('trakt_status_watched'))}</span>`;
      } else {
        btn.innerHTML = `${iconHtml('trakt-watched', 14)} <span>${esc(t('trakt_btn_mark_watched'))}</span>`;
        btn.onclick = (e) => {
          e.stopPropagation();
          handleMarkWatched(ep, btn, card, thumb);
        };
      }
      actions.appendChild(btn);
    }

    if (options.showWatchlistToggle) {
      const wlBtn = document.createElement('button');
      wlBtn.type = 'button';
      if (ep.onWatchlist) {
        wlBtn.className = 'btn trakt-wl-badge-btn is-on';
        wlBtn.disabled = true;
        wlBtn.innerHTML = `${iconHtml('trakt-watchlist-on', 15)} <span>${esc(t('trakt_watchlist_added'))}</span>`;
      } else {
        wlBtn.className = 'btn btn-blue trakt-wl-add-btn';
        wlBtn.innerHTML = `${iconHtml('trakt-watchlist-add', 15)} <span>${esc(t('trakt_btn_add_watchlist_short'))}</span>`;
        wlBtn.onclick = async (e) => {
          e.stopPropagation();
          wlBtn.disabled = true;
          try {
            await invoke('trakt_watchlist_add', {
              traktId: ep.showTraktId,
              mediaType: 'show',
              title: ep.showTitle,
              year: null,
              tmdbId: ep.showTmdbId || null,
              posterPath: ep.posterPath || null,
              titleHu: ep.showTitleHu || null,
            });
            ep.onWatchlist = true;
            wlBtn.className = 'btn trakt-wl-badge-btn is-on';
            wlBtn.innerHTML = `${iconHtml('trakt-watchlist-on', 15)} <span>${esc(t('trakt_watchlist_added'))}</span>`;
          } catch (err) {
            wlBtn.disabled = false;
            if (window.TraktUI) window.TraktUI.showTraktError(err);
          }
        };
      }
      actions.appendChild(wlBtn);
    }

    if (isPosterCard) {
      if (actions.children.length > 0) {
        info.appendChild(actions);
      }
      card.appendChild(info);
    } else {
      const rightCol = document.createElement('div');
      rightCol.className = 'trakt-card-right';
      rightCol.appendChild(info);
      if (actions.children.length > 0) {
        rightCol.appendChild(actions);
      }
      card.appendChild(rightCol);
    }

    card.onclick = (e) => {
      if (e.target.closest('button')) return;
      if (window.TraktUI) {
        window.TraktUI.openTraktItemModal({
          traktId: ep.showTraktId,
          tmdbId: ep.showTmdbId,
          mediaType: 'show',
          title: showTitle,
          originalTitle: ep.showTitle,
          overview: ep.overview,
          posterPath: ep.posterPath,
          network: ep.network,
          onWatchlist: ep.onWatchlist,
          subtitle: `${epCode}${epTitleSuffix}${rightMetaText ? ' · ' + rightMetaText : ''}`,
        });
      }
    };

    return card;
  }

  async function handleMarkWatched(ep, btn, card, thumb) {
    btn.disabled = true;
    btn.classList.add('is-pending');
    btn.innerHTML = `${iconHtml('trakt-pending', 16)} <span>${esc(t('trakt_status_pending'))}</span>`;

    try {
      const res = await invoke('trakt_mark_watched', {
        episodeTraktId: ep.episodeTraktId,
        movieTraktId: null,
        showTraktId: ep.showTraktId,
        showTmdbId: ep.showTmdbId || null,
        showTitle: ep.showTitle,
        episodeTitle: ep.episodeTitle || null,
        season: ep.season,
        episode: ep.number,
        posterPath: ep.posterPath || null,
        watchedAt: new Date().toISOString(),
      });

      ep.watched = true;
      card.classList.add('trakt-card-watched');
      btn.classList.remove('is-pending');

      if (res && res.queued) {
        btn.classList.add('is-queued');
        btn.innerHTML = `${iconHtml('trakt-pending', 16)} <span>${esc(t('trakt_status_pending'))}</span>`;
        if (window.TraktUI) {
          window.TraktUI.showToast(t('trakt_error_offline'));
        }
      } else {
        btn.classList.add('is-watched');
        btn.innerHTML = `${iconHtml('trakt-watched-done', 16)} <span>${esc(t('trakt_status_watched'))}</span>`;
      }

      if (thumb && !thumb.querySelector('.trakt-watched-thumb-badge')) {
        const badge = document.createElement('span');
        badge.className = 'trakt-watched-thumb-badge';
        badge.title = t('trakt_status_watched');
        badge.innerHTML = iconHtml('trakt-watched-done', 16);
        thumb.appendChild(badge);
      }

      prependToHistoryRow({
        historyId: -Date.now(),
        traktId: ep.episodeTraktId,
        mediaType: 'episode',
        showTraktId: ep.showTraktId,
        showTmdbId: ep.showTmdbId,
        showTitle: ep.showTitle,
        episodeTitle: ep.episodeTitle,
        season: ep.season,
        episode: ep.number,
        watchedAt: (res && res.watchedAt) || new Date().toISOString(),
        posterPath: ep.posterPath,
      });

      if (ep.isContinueWatching && !(res && res.queued) && window.TraktUI?.refreshDashboardIfActive) {
        setTimeout(() => {
          window.TraktUI.refreshDashboardIfActive();
        }, 650);
      }
    } catch (err) {
      btn.disabled = false;
      btn.classList.remove('is-pending');
      btn.innerHTML = `${iconHtml('trakt-watched', 16)} <span>${esc(t('trakt_btn_mark_watched'))}</span>`;
      if (window.TraktUI) window.TraktUI.showTraktError(err);
    }
  }

  function renderHistoryCard(item) {
    const card = document.createElement('div');
    card.className = 'trakt-poster-card trakt-card-history';

    const thumb = createThumbElement(
      item.posterPath,
      'w342',
      item.mediaType === 'movie' ? 'movie' : 'device-tv',
      item.showTmdbId,
      item.mediaType === 'movie' ? 'movie' : 'tv'
    );
    thumb.classList.add('trakt-poster-card-thumb');
    card.appendChild(thumb);

    const info = document.createElement('div');
    info.className = 'trakt-poster-card-body';

    const lang = typeof currentLang !== 'undefined' ? currentLang : 'hungarian';
    const relTime = window.TraktDates
      ? window.TraktDates.formatRelativeTime(item.watchedAt, lang)
      : item.watchedAt;
    const fullDate = item.watchedAt
      ? new Date(item.watchedAt).toLocaleString(lang === 'english' ? 'en-US' : 'hu-HU')
      : '';

    const isEp = item.mediaType === 'episode' && item.season != null && item.episode != null;
    const codeLine = isEp ? formatEpisodeCode(item.season, item.episode) : t('media_type_movie');
    const epNameLine = isEp ? (item.episodeTitle || '—') : '';
    const subLine = isEp
      ? `${codeLine}${item.episodeTitle ? ' – ' + item.episodeTitle : ''}`
      : t('media_type_movie');

    info.innerHTML = `
      <div class="trakt-pcard-show" title="${esc(item.showTitle)}">${esc(item.showTitle)}</div>
      <div class="trakt-pcard-code">
        <span>${esc(codeLine)}</span>
        <span class="trakt-pcard-time trakt-history-time" title="${esc(fullDate)}">${esc(relTime)}</span>
      </div>
      ${epNameLine ? `<div class="trakt-pcard-ep" title="${esc(epNameLine)}">${esc(epNameLine)}</div>` : ''}
    `;
    card.appendChild(info);

    card.onclick = () => {
      if (window.TraktUI) {
        window.TraktUI.openTraktItemModal({
          traktId: item.showTraktId || item.traktId,
          tmdbId: item.showTmdbId,
          mediaType: item.mediaType === 'movie' ? 'movie' : 'show',
          title: item.showTitle,
          originalTitle: item.showTitle,
          posterPath: item.posterPath,
          subtitle: `${subLine} · ${fullDate}`,
        });
      }
    };

    return card;
  }

  function prependToHistoryRow(item) {
    const row = document.getElementById('traktHistoryRowList');
    if (!row) return;
    const emptyMsg = row.querySelector('.trakt-empty-msg');
    if (emptyMsg) emptyMsg.remove();
    const card = renderHistoryCard(item);
    row.insertBefore(card, row.firstChild);
    while (row.children.length > 30) {
      row.removeChild(row.lastChild);
    }
  }

  function renderDashboard(viewport, data, options = {}) {
    const lang = typeof currentLang !== 'undefined' ? currentLang : 'hungarian';
    const now = new Date();
    const allCalendarEps = data.calendarEpisodes || [];
    const split = window.TraktDates.splitDashboardEpisodes(allCalendarEps, now);
    const furtherWeeks = window.TraktDates.getFurtherWeekRanges(now, 30);

    if (!options.preservePage) {
      furtherWeekIndex = 0;
    } else if (furtherWeekIndex >= furtherWeeks.length) {
      furtherWeekIndex = Math.max(0, furtherWeeks.length - 1);
    }

    const todayEpIds = new Set(split.today.map((e) => e.episodeTraktId));
    const todayShowEpKeys = new Set(
      split.today.map((e) => `${e.showTraktId}:${e.season}:${e.number}`)
    );
    const nowMs = now.getTime();
    const continueItems = (data.continueWatching || [])
      .filter((cw) => {
        if (
          todayEpIds.has(cw.episodeTraktId) ||
          todayShowEpKeys.has(`${cw.showTraktId}:${cw.season}:${cw.number}`)
        ) {
          return false;
        }
        if (!cw.firstAired) return false;
        const airedMs = new Date(cw.firstAired).getTime();
        return !isNaN(airedMs) && airedMs <= nowMs;
      })
      .map((cw) => ({
        ...cw,
        watched: false,
        isContinueWatching: true,
      }));
    const todayCombined = [
      ...split.today.map((ep) => ({ ...ep, isContinueWatching: false })),
      ...continueItems,
    ].sort(window.TraktDates.compareEpisodesByStartTime);

    viewport.innerHTML = `
      <div class="trakt-dashboard">
        <div class="trakt-dash-top">
          <section class="trakt-dash-panel trakt-dash-today">
            <div class="trakt-panel-header">
              <span class="trakt-panel-title">${esc(t('trakt_today_title'))}</span>
              <span class="trakt-panel-count">${todayCombined.length}</span>
            </div>
            <div class="trakt-panel-list custom-scroll" id="traktTodayList"></div>
          </section>
          <section class="trakt-dash-panel trakt-dash-tomorrow">
            <div class="trakt-panel-header">
              <span class="trakt-panel-title">${esc(t('trakt_tomorrow_title'))}</span>
              <span class="trakt-panel-count">${split.tomorrow.length}</span>
            </div>
            <div class="trakt-panel-list custom-scroll" id="traktTomorrowList"></div>
          </section>
        </div>

        <section class="trakt-dash-row trakt-dash-further">
          <div class="trakt-panel-header">
            <span class="trakt-panel-title">${esc(t('trakt_further_title'))}</span>
            <span class="trakt-panel-range" id="traktFurtherRange"></span>
            <div class="trakt-panel-header-end">
              <span class="trakt-panel-count" id="traktFurtherCount">0</span>
              <div class="trakt-week-pager" id="traktFurtherPager">
                <button type="button" class="trakt-pager-btn" id="traktFurtherPrevBtn" aria-label="${esc(t('trakt_pager_prev_week'))}">
                  ${iconHtml('chevron-left', 14)}
                </button>
                <span class="trakt-pager-indicator" id="traktFurtherPageIndicator"></span>
                <button type="button" class="trakt-pager-btn" id="traktFurtherNextBtn" aria-label="${esc(t('trakt_pager_next_week'))}">
                  ${iconHtml('chevron-right', 14)}
                </button>
              </div>
            </div>
          </div>
          <div class="trakt-hscroll-list custom-scroll" id="traktFurtherList"></div>
        </section>

        <section class="trakt-dash-row trakt-dash-history">
          <div class="trakt-panel-header">
            <span class="trakt-panel-icon">${iconHtml('trakt-history', 16)}</span>
            <span class="trakt-panel-title">${esc(t('trakt_history_title'))}</span>
            <span class="trakt-panel-count">${(data.history || []).length}</span>
          </div>
          <div class="trakt-hscroll-list custom-scroll" id="traktHistoryRowList"></div>
        </section>
      </div>
    `;

    ensureImageObserver(viewport);

    const todayList = document.getElementById('traktTodayList');
    if (todayCombined.length === 0) {
      todayList.innerHTML = `<div class="trakt-empty-msg">${esc(t('trakt_no_today_premieres'))}</div>`;
    } else {
      todayCombined.forEach((ep) =>
        todayList.appendChild(
          renderEpisodeCard(ep, {
            showWatched: true,
            posterCard: true,
            showDynamicPastDay: true,
            nowDate: now,
            isContinueWatching: !!ep.isContinueWatching,
          })
        )
      );
    }

    const tomorrowList = document.getElementById('traktTomorrowList');
    if (split.tomorrow.length === 0) {
      tomorrowList.innerHTML = `<div class="trakt-empty-msg">${esc(t('trakt_no_tomorrow_premieres'))}</div>`;
    } else {
      split.tomorrow.forEach((ep) =>
        tomorrowList.appendChild(
          renderEpisodeCard(ep, { showWatched: true, posterCard: true, nowDate: now })
        )
      );
    }

    const furtherList = document.getElementById('traktFurtherList');
    const furtherRangeEl = document.getElementById('traktFurtherRange');
    const furtherCountEl = document.getElementById('traktFurtherCount');
    const furtherPrevBtn = document.getElementById('traktFurtherPrevBtn');
    const furtherNextBtn = document.getElementById('traktFurtherNextBtn');
    const furtherPageIndEl = document.getElementById('traktFurtherPageIndicator');

    const renderFurtherWeekPage = (idx) => {
      if (furtherWeeks.length === 0) {
        if (furtherRangeEl) furtherRangeEl.textContent = '';
        if (furtherCountEl) furtherCountEl.textContent = '0';
        if (furtherPageIndEl) furtherPageIndEl.textContent = '0 / 0';
        if (furtherPrevBtn) furtherPrevBtn.disabled = true;
        if (furtherNextBtn) furtherNextBtn.disabled = true;
        furtherList.innerHTML = `<div class="trakt-empty-msg">${esc(t('trakt_no_further_premieres'))}</div>`;
        return;
      }

      furtherWeekIndex = Math.max(0, Math.min(idx, furtherWeeks.length - 1));
      const activeWeek = furtherWeeks[furtherWeekIndex];
      const activeRangeLabel = window.TraktDates.formatShortDateRange(
        activeWeek.start,
        activeWeek.end,
        lang
      );
      const weekEps = window.TraktDates.filterEpisodesByDateRange(
        allCalendarEps,
        activeWeek.startKey,
        activeWeek.endKey
      );

      if (furtherRangeEl) furtherRangeEl.textContent = activeRangeLabel;
      if (furtherCountEl) furtherCountEl.textContent = String(weekEps.length);
      if (furtherPageIndEl) {
        furtherPageIndEl.textContent = `${furtherWeekIndex + 1} / ${furtherWeeks.length}`;
      }

      if (furtherPrevBtn) {
        const hasPrev = furtherWeekIndex > 0;
        furtherPrevBtn.disabled = !hasPrev;
        if (hasPrev) {
          const prevW = furtherWeeks[furtherWeekIndex - 1];
          const prevLbl = window.TraktDates.formatShortDateRange(prevW.start, prevW.end, lang);
          furtherPrevBtn.title = `${t('trakt_pager_prev_week')} (${prevLbl})`;
        } else {
          furtherPrevBtn.title = t('trakt_pager_prev_week');
        }
      }

      if (furtherNextBtn) {
        const hasNext = furtherWeekIndex < furtherWeeks.length - 1;
        furtherNextBtn.disabled = !hasNext;
        if (hasNext) {
          const nextW = furtherWeeks[furtherWeekIndex + 1];
          const nextLbl = window.TraktDates.formatShortDateRange(nextW.start, nextW.end, lang);
          furtherNextBtn.title = `${t('trakt_pager_next_week')} (${nextLbl})`;
        } else {
          furtherNextBtn.title = t('trakt_pager_next_week');
        }
      }

      furtherList.innerHTML = '';
      furtherList.scrollLeft = 0;
      if (weekEps.length === 0) {
        furtherList.innerHTML = `<div class="trakt-empty-msg">${esc(t('trakt_no_further_premieres'))}</div>`;
      } else {
        weekEps.forEach((ep) =>
          furtherList.appendChild(
            renderEpisodeCard(ep, {
              showWatched: true,
              showDate: true,
              compact: true,
              posterCard: true,
              nowDate: now,
            })
          )
        );
      }
    };

    if (furtherPrevBtn) {
      furtherPrevBtn.onclick = () => renderFurtherWeekPage(furtherWeekIndex - 1);
    }
    if (furtherNextBtn) {
      furtherNextBtn.onclick = () => renderFurtherWeekPage(furtherWeekIndex + 1);
    }

    renderFurtherWeekPage(furtherWeekIndex);

    const historyList = document.getElementById('traktHistoryRowList');
    const histItems = data.history || [];
    if (histItems.length === 0) {
      historyList.innerHTML = `<div class="trakt-empty-msg">${esc(t('trakt_no_history'))}</div>`;
    } else {
      histItems.forEach((item) => historyList.appendChild(renderHistoryCard(item)));
    }
  }

  function renderWatchlistView(viewport, items, stateObj) {
    const filter = stateObj.filter || 'all';
    const sort = stateObj.sort || 'added';
    const query = (stateObj.query || '').trim();
    const queryLower = query.toLowerCase();
    const preparedQuery =
      query && window.SearchUtils ? window.SearchUtils.prepareQuery(query) : null;

    viewport.innerHTML = `
      <div class="trakt-subview">
        <div class="trakt-subview-toolbar">
          <h2 class="trakt-subview-title">${iconHtml('trakt-watchlist-on', 20)} ${esc(t('trakt_watchlist_title'))}</h2>
          <div class="trakt-filter-pills">
            <button class="btn trakt-pill ${filter === 'all' ? 'active' : ''}" data-wl-filter="all">${esc(t('filter_all'))}</button>
            <button class="btn trakt-pill ${filter === 'shows' ? 'active' : ''}" data-wl-filter="shows">${esc(t('trakt_filter_shows'))}</button>
            <button class="btn trakt-pill ${filter === 'movies' ? 'active' : ''}" data-wl-filter="movies">${esc(t('trakt_filter_movies'))}</button>
          </div>
          <input type="text" id="traktWlSearchInput" class="trakt-search-input" placeholder="${esc(t('search_placeholder'))}" value="${esc(stateObj.query || '')}" />
          <select id="traktWlSortSelect" class="trakt-sort-select">
            <option value="added" ${sort === 'added' ? 'selected' : ''}>${esc(t('trakt_sort_added'))}</option>
            <option value="title" ${sort === 'title' ? 'selected' : ''}>${esc(t('trakt_sort_title'))}</option>
            <option value="year" ${sort === 'year' ? 'selected' : ''}>${esc(t('trakt_sort_year'))}</option>
          </select>
        </div>
        <div class="trakt-watchlist-grid custom-scroll" id="traktWatchlistGrid"></div>
      </div>
    `;

    ensureImageObserver(viewport);

    const filtered = (items || []).filter((it) => {
      if (filter === 'shows' && it.mediaType !== 'show') return false;
      if (filter === 'movies' && it.mediaType !== 'movie') return false;
      if (query) {
        if (preparedQuery) {
          const titleMatch =
            window.SearchUtils.matchPrepared(it.title || '', preparedQuery) ||
            window.SearchUtils.matchPrepared(it.titleHu || '', preparedQuery) ||
            (it.aliases || []).some((a) =>
              window.SearchUtils.matchPrepared(a, preparedQuery),
            );
          if (!titleMatch) return false;
        } else {
          const hay = `${it.title || ''} ${it.titleHu || ''} ${(it.aliases || []).join(' ')}`.toLowerCase();
          if (!hay.includes(queryLower)) return false;
        }
      }
      return true;
    });

    filtered.sort((a, b) => {
      if (sort === 'title') {
        const ta = (a.titleHu || a.title || '').toLowerCase();
        const tb = (b.titleHu || b.title || '').toLowerCase();
        return ta.localeCompare(tb);
      }
      if (sort === 'year') {
        return (b.year || 0) - (a.year || 0);
      }
      return String(b.listedAt || '').localeCompare(String(a.listedAt || ''));
    });

    const grid = document.getElementById('traktWatchlistGrid');
    if (filtered.length === 0) {
      grid.innerHTML = `<div class="trakt-empty-msg">${esc(t('trakt_watchlist_empty'))}</div>`;
    } else {
      filtered.forEach((item) => {
        const card = document.createElement('div');
        card.className = 'trakt-wl-card';

        const thumb = createThumbElement(
          item.posterPath,
          'w342',
          item.mediaType === 'movie' ? 'movie' : 'device-tv',
          item.tmdbId,
          item.mediaType === 'movie' ? 'movie' : 'tv'
        );
        thumb.classList.add('trakt-wl-poster');
        card.appendChild(thumb);

        const displayTitle =
          typeof currentLang !== 'undefined' && currentLang === 'hungarian' && item.titleHu
            ? item.titleHu
            : item.title;

        const body = document.createElement('div');
        body.className = 'trakt-wl-body';
        body.innerHTML = `
          <div class="trakt-wl-title" title="${esc(displayTitle)}">${esc(displayTitle)}</div>
          <div class="trakt-wl-meta">
            <span class="tmdb-badge tmdb-badge-media">${esc(item.mediaType === 'show' ? t('media_type_tv') : t('media_type_movie'))}</span>
            ${item.year ? `<span class="tmdb-badge tmdb-badge-year">${item.year}</span>` : ''}
            ${item.rating ? `<span class="trakt-rating-mini">${iconHtml('trakt-rating-filled', 12)} ${item.rating.toFixed(1)}</span>` : ''}
          </div>
        `;

        const removeBtn = document.createElement('button');
        removeBtn.type = 'button';
        removeBtn.className = 'btn btn-red trakt-wl-remove-btn';
        removeBtn.innerHTML = `${iconHtml('trakt-watchlist-remove', 14)} <span>${esc(t('trakt_watchlist_remove'))}</span>`;
        removeBtn.onclick = async (e) => {
          e.stopPropagation();
          removeBtn.disabled = true;
          try {
            await invoke('trakt_watchlist_remove', {
              traktId: item.traktId,
              mediaType: item.mediaType,
            });
            const idx = items.findIndex((x) => x.traktId === item.traktId);
            if (idx !== -1) items.splice(idx, 1);
            renderWatchlistView(viewport, items, stateObj);
          } catch (err) {
            removeBtn.disabled = false;
            if (window.TraktUI) window.TraktUI.showTraktError(err);
          }
        };
        body.appendChild(removeBtn);
        card.appendChild(body);

        card.onclick = (e) => {
          if (e.target.closest('button')) return;
          if (window.TraktUI) {
            window.TraktUI.openTraktItemModal({
              traktId: item.traktId,
              tmdbId: item.tmdbId,
              mediaType: item.mediaType,
              title: displayTitle,
              originalTitle: item.title,
              aliases: item.aliases || [],
              year: item.year,
              rating: item.rating,
              votes: item.votes,
              status: item.status,
              network: item.network,
              overview: item.overview,
              posterPath: item.posterPath,
              onWatchlist: true,
            });
          }
        };

        grid.appendChild(card);
      });
    }

    viewport.querySelectorAll('[data-wl-filter]').forEach((btn) => {
      btn.onclick = () => {
        stateObj.filter = btn.dataset.wlFilter;
        renderWatchlistView(viewport, items, stateObj);
      };
    });

    const searchInput = document.getElementById('traktWlSearchInput');
    if (searchInput) {
      searchInput.oninput = (e) => {
        stateObj.query = e.target.value;
        renderWatchlistView(viewport, items, stateObj);
        const newInput = document.getElementById('traktWlSearchInput');
        if (newInput) {
          newInput.focus();
          newInput.setSelectionRange(newInput.value.length, newInput.value.length);
        }
      };
    }

    const sortSelect = document.getElementById('traktWlSortSelect');
    if (sortSelect) {
      sortSelect.onchange = (e) => {
        stateObj.sort = e.target.value;
        renderWatchlistView(viewport, items, stateObj);
      };
    }
  }

  function renderCalendarView(viewport, episodes, mode, options = {}) {
    const isGlobal = mode === 'global';
    const titleKey = isGlobal ? 'trakt_calendar_global_title' : 'trakt_calendar_my_title';
    const iconName = isGlobal ? 'trakt-calendar-global' : 'trakt-calendar-my';
    const lang = typeof currentLang !== 'undefined' ? currentLang : 'hungarian';

    const todayKey = window.TraktDates.toLocalYmd(new Date());
    const maxDays = isGlobal ? 14 : 30;
    const maxKey = window.TraktDates.toLocalYmd(window.TraktDates.addLocalDays(new Date(), maxDays));

    const filtered = (episodes || []).filter((ep) => {
      if (!ep.firstAired) return false;
      const k = window.TraktDates.toLocalYmd(new Date(ep.firstAired));
      return k >= todayKey && k <= maxKey;
    });

    const grouped = window.TraktDates.groupByLocalDay(filtered);
    const sortedDays = Array.from(grouped.keys()).sort();

    let globalFilterHtml = '';
    if (isGlobal && options.onChangeFilters) {
      const f = options.filters || { countries: '', languages: '' };
      const curKey = `${f.countries || ''}|${f.languages || ''}`;
      const presets = [
        { key: '|', label: t('trakt_cal_filter_all') },
        { key: 'us,gb|en', label: t('trakt_cal_filter_us_gb') },
        { key: 'us,gb,hu|en,hu', label: t('trakt_cal_filter_us_gb_hu') },
        { key: 'hu|hu', label: t('trakt_cal_filter_hu') },
      ];
      if (!presets.some((p) => p.key === curKey)) {
        presets.push({ key: curKey, label: `${f.countries || '*'} / ${f.languages || '*'}` });
      }
      globalFilterHtml = `
        <select id="traktGlobalCalFilterSelect" class="trakt-select" title="${esc(t('trakt_cal_filter_title'))}">
          ${presets
            .map(
              (p) =>
                `<option value="${esc(p.key)}" ${p.key === curKey ? 'selected' : ''}>${esc(p.label)}</option>`
            )
            .join('')}
        </select>
      `;
    }

    viewport.innerHTML = `
      <div class="trakt-subview">
        <div class="trakt-subview-toolbar">
          <h2 class="trakt-subview-title">${iconHtml(iconName, 20)} ${esc(t(titleKey))}</h2>
          <span class="trakt-cal-active-day u-hidden" id="traktCalActiveDay"></span>
          ${globalFilterHtml}
          <span class="trakt-panel-count">${filtered.length}</span>
        </div>
        <div class="trakt-calendar-days custom-scroll" id="traktCalendarDays"></div>
      </div>
    `;

    if (isGlobal && options.onChangeFilters) {
      const sel = document.getElementById('traktGlobalCalFilterSelect');
      if (sel) {
        sel.onchange = (e) => {
          const [countries = '', languages = ''] = String(e.target.value || '|').split('|');
          options.onChangeFilters({ countries, languages });
        };
      }
    }

    ensureImageObserver(viewport);
    const container = document.getElementById('traktCalendarDays');

    if (sortedDays.length === 0) {
      container.innerHTML = `<div class="trakt-empty-msg">${esc(t('trakt_calendar_empty'))}</div>`;
      return;
    }

    sortedDays.forEach((dayKey) => {
      const dayEps = [...(grouped.get(dayKey) || [])];
      if (isGlobal) {
        const cmpLocale = lang === 'english' ? 'en' : 'hu';
        dayEps.sort((a, b) => {
          const titleCmp = getDisplayShowTitle(a).localeCompare(
            getDisplayShowTitle(b),
            cmpLocale,
            { sensitivity: 'base', numeric: true }
          );
          if (titleCmp !== 0) return titleCmp;
          if ((a.season || 0) !== (b.season || 0)) return (a.season || 0) - (b.season || 0);
          if ((a.number || 0) !== (b.number || 0)) return (a.number || 0) - (b.number || 0);
          return String(a.firstAired || '').localeCompare(String(b.firstAired || ''));
        });
      }
      const section = document.createElement('div');
      section.className = 'trakt-cal-day-group';

      const heading = window.TraktDates.formatDayHeading(dayKey, lang, {
        today: t('day_today'),
        tomorrow: t('day_tomorrow'),
      });
      section.dataset.dayHeading = heading;

      section.innerHTML = `<div class="trakt-cal-day-header">${esc(heading)}</div>`;
      const list = document.createElement('div');
      list.className = 'trakt-cal-day-list';

      dayEps.forEach((ep) => {
        list.appendChild(
          renderEpisodeCard(ep, {
            showWatched: !isGlobal,
            showWatchlistToggle: isGlobal,
          })
        );
      });

      section.appendChild(list);
      container.appendChild(section);
    });

    if (window.TraktDates && typeof window.TraktDates.attachStickyDayHeader === 'function') {
      window.TraktDates.attachStickyDayHeader(
        container,
        document.getElementById('traktCalActiveDay'),
        { groupSelector: '.trakt-cal-day-group' }
      );
    }
  }

  async function renderProgressAndAiringsInto(containerEl, item, isStale) {
    if (!containerEl) return;
    containerEl.classList.remove('u-hidden');
    containerEl.innerHTML = `<div class="trakt-drawer-loading"><span class="spinner-small"></span> ${esc(t('trakt_loading'))}</div>`;

    if (!item.traktId && !item.showTraktId) {
      containerEl.classList.add('u-hidden');
      containerEl.innerHTML = '';
      return;
    }

    const showTraktId = item.traktId || item.showTraktId;
    const isShow = item.mediaType === 'show' || item.mediaType === 'tv';
    const lang = typeof currentLang !== 'undefined' && currentLang === 'english' ? 'en-US' : 'hu-HU';
    const titlesToSearch = Array.from(
      new Set([item.title, item.originalTitle, ...(item.aliases || [])].filter(Boolean))
    );

    const [progressRes, airingsRes] = await Promise.allSettled([
      isShow
        ? invoke('trakt_get_show_seasons_progress', {
            showTraktId,
            tmdbId: item.tmdbId || null,
            lang,
          })
        : Promise.resolve(null),
      invoke('trakt_find_epg_airings', { titles: titlesToSearch }),
    ]);

    if (typeof isStale === 'function' && isStale()) return;
    if (!containerEl.isConnected) return;

    if (progressRes.status === 'rejected') {
      console.warn('[Trakt] trakt_get_show_seasons_progress error:', progressRes.reason);
    }
    if (airingsRes.status === 'rejected') {
      console.warn('[Trakt] trakt_find_epg_airings error:', airingsRes.reason);
    }

    const progress = progressRes.status === 'fulfilled' ? progressRes.value : null;
    const airings = airingsRes.status === 'fulfilled' ? airingsRes.value || [] : [];

    let html = '';

    if (progress && progress.seasons && progress.seasons.length > 0) {
      html += `
        <div class="trakt-drawer-block">
          <div class="trakt-drawer-header">
            <span>${esc(t('trakt_seasons_progress_title'))}</span>
            <span class="trakt-progress-summary">${progress.completed} / ${progress.aired}</span>
          </div>
          <div class="trakt-drawer-split">
            <div class="trakt-seasons-sidebar custom-scroll" id="traktDrawerSeasons"></div>
            <div class="trakt-episodes-list custom-scroll" id="traktDrawerEpisodes"></div>
          </div>
        </div>
      `;
    }

    html += `
      <div class="trakt-drawer-block">
        <div class="trakt-drawer-header">
          <span>${esc(t('trakt_epg_airings_title'))}</span>
          <span class="trakt-panel-count">${airings.length}</span>
        </div>
        <div class="trakt-epg-airings-list custom-scroll" id="traktDrawerAirings"></div>
      </div>
    `;

    containerEl.innerHTML = html;

    if (progress && progress.seasons && progress.seasons.length > 0) {
      const seasonsEl = document.getElementById('traktDrawerSeasons');
      const episodesEl = document.getElementById('traktDrawerEpisodes');
      let activeSeasonIdx = progress.seasons.findIndex((s) => s.seasonNumber > 0);
      if (activeSeasonIdx < 0) activeSeasonIdx = 0;

      const renderSeasonEpisodes = (season) => {
        episodesEl.innerHTML = '';
        (season.episodes || []).forEach((ep) => {
          const row = document.createElement('div');
          row.className = `trakt-ep-row ${ep.watched ? 'is-watched' : ''}`;
          const code = formatEpisodeCode(ep.season, ep.episode);
          const placeholderHtml = `<span class="trakt-thumb-placeholder${ep.stillPath ? ' u-hidden' : ''}">${iconHtml('photo', 20)}</span>`;
          const stillHtml = `<div class="trakt-ep-still-wrap">${
            ep.stillPath
              ? `<img class="trakt-ep-still" src="https://image.tmdb.org/t/p/w300${esc(ep.stillPath)}" alt="" loading="lazy" />`
              : ''
          }${placeholderHtml}</div>`;
          row.innerHTML = `
            <span class="trakt-ep-status-icon">${iconHtml(ep.watched ? 'trakt-watched-done' : 'trakt-watched', 18)}</span>
            ${stillHtml}
            <div class="trakt-ep-text">
              <div class="trakt-ep-title">${esc(code)} – ${esc(ep.title)}</div>
              ${ep.overview ? `<div class="trakt-ep-overview">${esc(ep.overview)}</div>` : ''}
            </div>
          `;
          const stillImg = row.querySelector('.trakt-ep-still');
          if (stillImg) {
            stillImg.onerror = () => {
              stillImg.classList.add('u-hidden');
              const ph = row.querySelector('.trakt-ep-still-wrap .trakt-thumb-placeholder');
              if (ph) ph.classList.remove('u-hidden');
            };
          }
          episodesEl.appendChild(row);
        });
      };

      progress.seasons.forEach((s, idx) => {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = `trakt-season-btn ${idx === activeSeasonIdx ? 'active' : ''}`;
        btn.innerHTML = `
          <span>${esc(s.name)}</span>
          <span class="trakt-season-count">${s.completed}/${s.aired}</span>
        `;
        btn.onclick = () => {
          seasonsEl.querySelectorAll('.trakt-season-btn').forEach((b) => b.classList.remove('active'));
          btn.classList.add('active');
          renderSeasonEpisodes(s);
        };
        seasonsEl.appendChild(btn);
      });

      renderSeasonEpisodes(progress.seasons[activeSeasonIdx]);
    }

    const airingsEl = document.getElementById('traktDrawerAirings');
    if (airings.length === 0) {
      airingsEl.innerHTML = `<div class="trakt-empty-msg">${esc(t('trakt_no_epg_airings'))}</div>`;
    } else {
      const lang = typeof currentLang !== 'undefined' ? currentLang : 'hungarian';
      const locale = lang === 'english' ? 'en-US' : 'hu-HU';
      const todayMid = new Date();
      todayMid.setHours(0, 0, 0, 0);
      const daysArr = typeof t === 'function' ? t('days') : null;

      const updateFavButtonsForTitle = (progTitle, isFav) => {
        airingsEl.querySelectorAll('.trakt-airing-fav-btn').forEach((btn) => {
          if (btn.dataset.progTitle === progTitle) {
            btn.classList.toggle('active', isFav);
            btn.title = isFav ? t('trakt_btn_fav_remove') : t('trakt_btn_fav_add');
            btn.innerHTML = iconHtml(isFav ? 'star-filled' : 'star', 15);
          }
        });
      };

      airings.forEach((a) => {
        const startDate = typeof parseEPGDate === 'function' ? parseEPGDate(a.start, a.timeOffset || 0) : new Date(a.startTs * 1000);
        const fullDateStr = startDate.toLocaleDateString(locale, {
          year: 'numeric',
          month: 'long',
          day: 'numeric',
        });

        const targetMid = new Date(startDate.getFullYear(), startDate.getMonth(), startDate.getDate());
        const diffDays = Math.round((targetMid - todayMid) / 86400000);
        let dayPillText =
          Array.isArray(daysArr) && daysArr[startDate.getDay()]
            ? daysArr[startDate.getDay()]
            : startDate.toLocaleDateString(locale, { weekday: 'long' });
        let dayPillClass = 'trakt-airing-day-pill';
        if (diffDays === 0) {
          dayPillText = t('day_today');
          dayPillClass += ' is-today';
        } else if (diffDays === 1) {
          dayPillText = t('day_tomorrow');
          dayPillClass += ' is-tomorrow';
        } else if (diffDays === -1) {
          dayPillText = t('day_yesterday');
        }

        const startTimeStr = `${pad2(startDate.getHours())}:${pad2(startDate.getMinutes())}`;
        const isFav = window.EpgApp ? window.EpgApp.isFavoriteTitle(a.title) : false;

        const row = document.createElement('div');
        row.className = 'trakt-airing-row';
        row.title = a.title || '';
        row.innerHTML = `
          <button type="button" class="trakt-airing-icon-btn trakt-airing-jump-btn" title="${esc(t('trakt_btn_jump_epg'))}${a.title ? ' – ' + esc(a.title) : ''}">
            ${iconHtml('trakt-jump-epg', 15)}
          </button>
          <button type="button" class="trakt-airing-icon-btn trakt-airing-fav-btn ${isFav ? 'active' : ''}" title="${esc(isFav ? t('trakt_btn_fav_remove') : t('trakt_btn_fav_add'))}">
            ${iconHtml(isFav ? 'star-filled' : 'star', 15)}
          </button>
          <span class="trakt-airing-date" title="${esc(fullDateStr)}">${esc(fullDateStr)}</span>
          <span class="${dayPillClass}" title="${esc(dayPillText)}">${esc(dayPillText)}</span>
          <span class="start-badge trakt-airing-time-pill">${esc(startTimeStr)}</span>
          <span class="channel-badge trakt-airing-channel" title="${esc(a.channelName)}">${esc(a.channelName)}</span>
          <span class="channel-source-badge trakt-airing-source" title="${esc(a.sourceName)}">${esc(a.sourceName)}</span>
        `;

        const jumpBtn = row.querySelector('.trakt-airing-jump-btn');
        if (jumpBtn) {
          jumpBtn.onclick = (e) => {
            e.stopPropagation();
            if (window.EpgApp && typeof window.EpgApp.jumpToEpgAiring === 'function') {
              window.EpgApp.jumpToEpgAiring(a);
            }
          };
        }

        const favBtn = row.querySelector('.trakt-airing-fav-btn');
        if (favBtn) {
          favBtn.dataset.progTitle = a.title || '';
          favBtn.onclick = async (e) => {
            e.stopPropagation();
            if (!a.title || !window.EpgApp || typeof window.EpgApp.toggleFavorite !== 'function') return;
            favBtn.disabled = true;
            try {
              const newFav = await window.EpgApp.toggleFavorite(a.title);
              updateFavButtonsForTitle(a.title, !!newFav);
            } finally {
              favBtn.disabled = false;
            }
          };
        }

        airingsEl.appendChild(row);
      });
    }
  }

  window.TraktDashboard = {
    renderDashboard,
    renderWatchlistView,
    renderCalendarView,
    renderProgressAndAiringsInto,
    prependToHistoryRow,
  };
})();
