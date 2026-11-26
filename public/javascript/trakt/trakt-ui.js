/**
 * Trakt.tv UI Controller (Header, View Toggle, Auth, Modal, EPG Matcher, Settings)
 */
import { state, invoke, listen, hasTauri } from '../application/state.js';
import {
  t,
  getCurrentLang,
  addLanguageChangeListener,
} from '../application/i18n.js';
import { Icons } from '../utils/icons.js';
import { TraktDates } from './trakt-dates.js';
import { iconHtml, showConfirm, showAlert } from '../application/ui.js';
import { renderEPG } from '../application/epg.js';
import {
  updateModalLeftColVisibility,
  resetModalTmdbUi,
  renderTmdbDetailsInModal,
  closeFixOverlay,
} from '../tmdb/tmdb.js';
import { updateModalFavStar, setModalContext } from '../application/modal.js';
import { detachGoogleCalendarFromModal } from '../google/google-ui.js';
import {
  renderDashboard,
  renderWatchlistView,
  renderCalendarView,
  renderProgressAndAiringsInto,
} from './trakt-dashboard.js';
import {
  renderWatchlistEpgView,
  refreshWatchlistEpgIfActive,
} from './trakt-watchlist-epg.js';

  let traktModeActive = false;
  let currentSubView = 'dashboard'; // 'dashboard' | 'watchlist' | 'calendar_my' | 'calendar_global'
  let authStatus = { loggedIn: false, enabled: true, profile: null };
  let syncStatus = {
    lastSyncOkAt: null,
    overdue: true,
    pendingCount: 0,
    status: 'idle',
    step: '',
    current: 0,
    total: 0,
    isFull: false,
    offline: false,
  };
  let watchlistTitleSet = new Set();
  let watchlistState = { filter: 'all', sort: 'added', query: '' };
  let activeDeviceFlow = null;
  let loginPollFallbackTimer = null;
  let modalAttachSeq = 0;
  let viewLoadSeq = 0;
  let manualSyncInFlight = false;
  let cachedAvatarUrl = null;
  let cachedAvatarDataUri = null;

  function stopLoginFallbackTimer() {
    if (loginPollFallbackTimer) {
      clearInterval(loginPollFallbackTimer);
      loginPollFallbackTimer = null;
    }
  }

  async function handleLoginSuccess(profile) {
    stopLoginFallbackTimer();
    activeDeviceFlow = null;
    authStatus.loggedIn = true;
    if (profile) authStatus.profile = profile;
    try {
      const fresh = await invoke('trakt_get_auth_status');
      authStatus = fresh;
      syncStatus = { ...syncStatus, ...(await invoke('trakt_get_sync_status')) };
    } catch (_) {}
    updateHeaderVisibility();
    updateSyncStatusUi();
    if (traktModeActive) await loadActiveTraktView();
    if (document.getElementById('tabPaneTrakt')?.classList.contains('active')) {
      await loadSettingsTab();
    }
  }

  function startLoginFallbackTimer() {
    stopLoginFallbackTimer();
    loginPollFallbackTimer = setInterval(async () => {
      if (!activeDeviceFlow) {
        stopLoginFallbackTimer();
        return;
      }
      try {
        const st = await invoke('trakt_get_auth_status');
        if (st && st.loggedIn) {
          await handleLoginSuccess(st.profile);
        }
      } catch (_) {}
    }, 3000);
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

  function formatTraktError(err) {
    const s = String(err || '');
    if (s.includes('TRAKT_ERR_420')) return t('trakt_error_watchlist_limit');
    if (s.includes('TRAKT_ERR_426')) return t('trakt_error_vip_required');
    if (s.includes('TRAKT_ERR_423')) return t('trakt_error_locked_account');
    if (s.includes('TRAKT_NOT_FOUND')) return t('trakt_error_not_found');
    if (s.includes('TRAKT_OFFLINE')) return t('trakt_error_offline');
    if (s.includes('TRAKT_AUTH_EXPIRED')) return t('trakt_error_auth_expired');
    if (s.includes('TRAKT_NOT_LOGGED_IN')) return t('trakt_error_not_logged_in');
    return s;
  }

  function showTraktError(err) {
    const msg = formatTraktError(err);
    if (typeof showAlert === 'function') {
      showAlert(t('alert_error_title'), msg);
    } else {
      alert(msg);
    }
  }

  function showToast(msg) {
    const statusEl = document.getElementById('traktSyncStatus');
    if (statusEl) {
      statusEl.textContent = msg;
    }
  }

  let lastTraktBtnWidth = 0;

  function syncHeaderModeButtonWidth() {
    const traktBtn = document.getElementById('traktBtn');
    if (!traktBtn) return;
    if (!traktBtn.classList.contains('u-hidden')) {
      const w = traktBtn.getBoundingClientRect().width;
      if (w > 0) lastTraktBtnWidth = Math.ceil(w);
    }
    if (lastTraktBtnWidth > 0) {
      document.documentElement.style.setProperty(
        '--trakt-mode-btn-width',
        `${lastTraktBtnWidth}px`
      );
    }
  }

  function updateWatchlistCountBadge() {
    const badge = document.getElementById('traktWatchlistCountBadge');
    if (!badge) return;
    const count = watchlistTitleSet.size;
    if (count > 0 && authStatus.loggedIn && authStatus.enabled) {
      badge.textContent = count > 99 ? '99+' : String(count);
      badge.classList.remove('u-hidden');
    } else {
      badge.classList.add('u-hidden');
    }
    syncHeaderModeButtonWidth();
  }

  function updateHeaderVisibility() {
    const traktBtn = document.getElementById('traktBtn');
    const backBtn = document.getElementById('traktBackBtn');
    syncHeaderModeButtonWidth();
    if (!authStatus.enabled) {
      if (traktBtn) traktBtn.classList.add('u-hidden');
      if (backBtn) backBtn.classList.add('u-hidden');
      if (traktModeActive) exitTraktMode();
    } else {
      if (traktBtn) traktBtn.classList.toggle('u-hidden', traktModeActive);
      if (backBtn) backBtn.classList.toggle('u-hidden', !traktModeActive);
    }
    updateWatchlistCountBadge();
  }

  function formatSyncProgressText() {
    if (syncStatus.status === 'stopping') {
      return t('trakt_sync_stopping');
    }
    if (syncStatus.status === 'cancelled') {
      return t('trakt_sync_stopped');
    }
    const base = syncStatus.isFull ? t('trakt_sync_full_in_progress') : t('trakt_sync_in_progress');
    const stepKeyMap = {
      pending: 'trakt_sync_step_pending',
      activities: 'trakt_sync_step_activities',
      watchlist: 'trakt_sync_step_watchlist',
      history: 'trakt_sync_step_history',
      calendar: 'trakt_sync_step_calendar',
      progress: 'trakt_sync_step_progress',
      tmdb: 'trakt_sync_step_tmdb',
    };
    const stepKey = syncStatus.step ? stepKeyMap[syncStatus.step] : null;
    const stepLabel = stepKey ? t(stepKey) : '';
    if (stepLabel && syncStatus.total > 0 && syncStatus.current > 0) {
      return `${base} (${stepLabel}: ${syncStatus.current}/${syncStatus.total})`;
    }
    if (stepLabel) {
      return `${base} (${stepLabel})`;
    }
    return base;
  }

  function updateSyncStatusUi() {
    const syncBtn = document.getElementById('traktSyncBtn');
    const statusEl = document.getElementById('traktSyncStatus');
    const isSyncing = syncStatus.status === 'syncing';
    const isStopping = syncStatus.status === 'stopping';
    const isBusy = isSyncing || isStopping;

    if (syncBtn) {
      const iconSpan = syncBtn.querySelector('[data-icon]');
      if (isBusy) {
        syncBtn.classList.add('trakt-sync-spinning');
        syncBtn.disabled = true;
        if (iconSpan) {
          iconSpan.setAttribute('data-icon', 'trakt-sync-active');
          if (Icons) Icons.replace(syncBtn);
        }
      } else {
        syncBtn.classList.remove('trakt-sync-spinning');
        syncBtn.disabled = false;
        if (iconSpan) {
          iconSpan.setAttribute('data-icon', 'trakt-sync');
          if (Icons) Icons.replace(syncBtn);
        }
      }
    }

    // Settings > Trakt.tv "full sync" / "stop" button
    const settingsSyncBtn = document.getElementById('traktSettingsSyncBtn');
    if (settingsSyncBtn) {
      if (isSyncing) {
        settingsSyncBtn.className = 'btn btn-red';
        settingsSyncBtn.disabled = false;
        settingsSyncBtn.title = t('trakt_btn_stop_sync_hint');
        settingsSyncBtn.innerHTML = `${iconHtml('player-stop', 16)} <span>${esc(t('trakt_btn_stop_sync'))}</span>`;
      } else if (isStopping) {
        settingsSyncBtn.className = 'btn btn-red';
        settingsSyncBtn.disabled = true;
        settingsSyncBtn.title = t('trakt_sync_stopping');
        settingsSyncBtn.innerHTML = `${iconHtml('player-stop', 16)} <span>${esc(t('trakt_sync_stopping'))}</span>`;
      } else {
        settingsSyncBtn.className = 'btn btn-blue';
        settingsSyncBtn.disabled = false;
        settingsSyncBtn.title = t('trakt_btn_full_sync_hint');
        settingsSyncBtn.innerHTML = `${iconHtml('trakt-sync', 16)} <span>${esc(t('trakt_btn_full_sync'))}</span>`;
      }
    }

    const clearCacheBtn = document.getElementById('traktClearCacheBtn');
    if (clearCacheBtn) clearCacheBtn.disabled = isBusy;
    const logoutBtn = document.getElementById('traktSettingsLogoutBtn');
    if (logoutBtn) logoutBtn.disabled = isBusy;

    const locale = getCurrentLang() === 'english' ? 'en-US' : 'hu-HU';
    let formattedLastSync = '';
    let isOverdue = false;
    if (syncStatus.lastSyncOkAt) {
      const d = new Date(syncStatus.lastSyncOkAt);
      formattedLastSync = isNaN(d.getTime()) ? syncStatus.lastSyncOkAt : d.toLocaleString(locale);
      isOverdue = !isNaN(d.getTime()) && Date.now() - d.getTime() > 24 * 3600 * 1000;
    }

    // Settings > Trakt.tv last-sync & live progress lines
    const settingsSyncEl = document.getElementById('traktSettingsSyncStatus');
    if (settingsSyncEl) {
      settingsSyncEl.classList.remove('is-overdue');
      if (!syncStatus.lastSyncOkAt) {
        settingsSyncEl.classList.add('is-overdue');
        settingsSyncEl.textContent = `${t('trakt_sync_last_ok')} ${t('trakt_sync_never')}`;
      } else {
        if (isOverdue) settingsSyncEl.classList.add('is-overdue');
        settingsSyncEl.textContent = `${t('trakt_sync_last_ok')} ${formattedLastSync}`;
      }
    }

    const settingsLiveEl = document.getElementById('traktSettingsSyncLive');
    if (settingsLiveEl) {
      settingsLiveEl.classList.remove('is-syncing', 'is-stopped', 'is-offline');
      if (isBusy) {
        settingsLiveEl.classList.add('is-syncing');
        settingsLiveEl.classList.remove('u-hidden');
        settingsLiveEl.innerHTML = `<span class="spinner-small"></span> <span>${esc(formatSyncProgressText())}</span>`;
      } else if (syncStatus.status === 'cancelled') {
        settingsLiveEl.classList.add('is-stopped');
        settingsLiveEl.classList.remove('u-hidden');
        settingsLiveEl.textContent = t('trakt_sync_stopped');
      } else if (syncStatus.offline || syncStatus.status === 'offline') {
        settingsLiveEl.classList.add('is-offline');
        settingsLiveEl.classList.remove('u-hidden');
        settingsLiveEl.textContent = t('trakt_status_offline');
      } else {
        settingsLiveEl.classList.add('u-hidden');
        settingsLiveEl.textContent = '';
      }
    }

    if (statusEl) {
      statusEl.classList.remove('is-overdue', 'is-offline');
      if (isBusy) {
        statusEl.textContent = formatSyncProgressText();
      } else if (syncStatus.status === 'cancelled') {
        statusEl.textContent = t('trakt_sync_stopped');
      } else if (syncStatus.offline || syncStatus.status === 'offline') {
        statusEl.classList.add('is-offline');
        statusEl.textContent = t('trakt_status_offline');
      } else if (!authStatus.loggedIn) {
        statusEl.textContent = '';
      } else if (!syncStatus.lastSyncOkAt) {
        statusEl.classList.add('is-overdue');
        statusEl.textContent = t('trakt_sync_never');
      } else if (isOverdue) {
        statusEl.classList.add('is-overdue');
        statusEl.textContent = `${t('trakt_sync_overdue')} (${formattedLastSync})`;
      } else {
        statusEl.textContent = `${t('trakt_sync_last_ok')} ${formattedLastSync}`;
      }
    }
  }

  function updateSubViewButtons() {
    const map = {
      watchlist: 'traktWatchlistViewBtn',
      watchlist_epg: 'traktWatchlistEpgBtn',
      calendar_my: 'traktMyCalendarBtn',
      calendar_global: 'traktGlobalCalendarBtn',
    };
    Object.entries(map).forEach(([key, id]) => {
      const btn = document.getElementById(id);
      if (btn) {
        if (currentSubView === key) btn.classList.add('active');
        else btn.classList.remove('active');
      }
    });
  }

  function renderLoginCard(container) {
    container.innerHTML = `
      <div class="trakt-login-wrapper">
        <div class="trakt-login-card">
          <img src="images/trakt/logo.svg" alt="Trakt" class="trakt-login-logo" />
          <h2>${esc(t('trakt_device_code_title'))}</h2>
          <p class="trakt-login-desc">${esc(t('trakt_login_intro'))}</p>
          <div id="traktDeviceFlowArea">
            <button type="button" class="btn btn-red trakt-start-login-btn" id="traktStartLoginBtn">
              ${iconHtml('trakt-login', 18)} <span>${esc(t('trakt_btn_login'))}</span>
            </button>
          </div>
          <div id="traktLoginError" class="trakt-login-error u-hidden"></div>
        </div>
      </div>
    `;

    const startBtn = container.querySelector('#traktStartLoginBtn');
    const flowArea = container.querySelector('#traktDeviceFlowArea');
    const errEl = container.querySelector('#traktLoginError');

    const showCodeUi = (data) => {
      activeDeviceFlow = data;
      startLoginFallbackTimer();
      const verifyUrl = data.verificationUrl || 'https://auth.trakt.tv/activate';
      flowArea.innerHTML = `
        <p class="trakt-device-desc">${esc(t('trakt_device_code_desc'))}</p>
        <div class="trakt-user-code" id="traktUserCodeDisplay">${esc(data.userCode)}</div>
        <div class="trakt-device-actions">
          <button type="button" class="btn btn-blue" id="traktCopyCodeBtn">
            ${iconHtml('copy', 16)} <span>${esc(t('trakt_btn_copy_code'))}</span>
          </button>
          <button type="button" class="btn btn-green" id="traktOpenActivateBtn">
            ${iconHtml('external-link', 16)} <span>${esc(t('trakt_btn_open_activation'))}</span>
          </button>
        </div>
        <div class="trakt-device-waiting">
          <span class="spinner-small"></span>
          <span>${esc(t('trakt_device_waiting'))}</span>
        </div>
      `;

      const copyBtn = flowArea.querySelector('#traktCopyCodeBtn');
      if (copyBtn) {
        copyBtn.onclick = () => {
          if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(data.userCode).then(() => {
              copyBtn.querySelector('span').textContent = t('trakt_code_copied');
              setTimeout(() => {
                if (copyBtn.isConnected) {
                  copyBtn.querySelector('span').textContent = t('trakt_btn_copy_code');
                }
              }, 1800);
            });
          }
        };
      }

      const openBtn = flowArea.querySelector('#traktOpenActivateBtn');
      if (openBtn) {
        openBtn.onclick = () => {
          if (
            verifyUrl.startsWith('https://auth.trakt.tv/') ||
            verifyUrl.startsWith('https://trakt.tv/')
          ) {
            invoke('open_url', { url: verifyUrl });
          }
        };
      }
    };

    if (activeDeviceFlow) {
      showCodeUi(activeDeviceFlow);
    }

    if (startBtn) {
      startBtn.onclick = async () => {
        startBtn.disabled = true;
        errEl.classList.add('u-hidden');
        try {
          const data = await invoke('trakt_start_login');
          showCodeUi(data);
        } catch (err) {
          startBtn.disabled = false;
          errEl.textContent = formatTraktError(err);
          errEl.classList.remove('u-hidden');
        }
      };
    }
  }

  async function loadActiveTraktView() {
    const seq = ++viewLoadSeq;
    const viewport = document.getElementById('traktViewport');
    if (!viewport) return;

    updateSubViewButtons();

    if (!authStatus.loggedIn && currentSubView !== 'calendar_global') {
      renderLoginCard(viewport);
      return;
    }

    viewport.innerHTML = `<div class="trakt-loading"><span class="spinner-small"></span> ${esc(t('trakt_loading'))}</div>`;

    try {
      const startDate = TraktDates.getLocalYesterdayYmd();
      if (currentSubView === 'dashboard') {
        const data = await invoke('trakt_get_dashboard_data', {
          startDate,
          forceRefresh: false,
        });
        if (seq !== viewLoadSeq || !traktModeActive) return;
        syncStatus.lastSyncOkAt = data.lastSyncOkAt;
        syncStatus.overdue = data.overdue;
        syncStatus.pendingCount = data.pendingCount;
        syncStatus.offline = data.offline;
        updateSyncStatusUi();
        renderDashboard(viewport, data);
      } else if (currentSubView === 'watchlist') {
        const items = await invoke('trakt_get_watchlist', { filter: null });
        if (seq !== viewLoadSeq || !traktModeActive) return;
        renderWatchlistView(viewport, items, watchlistState);
      } else if (currentSubView === 'watchlist_epg') {
        await renderWatchlistEpgView(viewport);
      } else if (currentSubView === 'calendar_my') {
        const eps = await invoke('trakt_get_calendar_my', {
          startDate,
          forceRefresh: false,
        });
        if (seq !== viewLoadSeq || !traktModeActive) return;
        renderCalendarView(viewport, eps, 'my');
      } else if (currentSubView === 'calendar_global') {
        const [eps, filters] = await Promise.all([
          invoke('trakt_get_calendar_global', {
            startDate,
            forceRefresh: false,
          }),
          invoke('trakt_get_global_calendar_filters').catch(() => ({ countries: '', languages: '' })),
        ]);
        if (seq !== viewLoadSeq || !traktModeActive) return;
        const handleGlobalFilterChange = async (nextFilters) => {
          const filterSeq = ++viewLoadSeq;
          try {
            await invoke('trakt_set_global_calendar_filters', {
              countries: nextFilters.countries || '',
              languages: nextFilters.languages || '',
            });
            if (filterSeq !== viewLoadSeq || !traktModeActive) return;
            viewport.innerHTML = `<div class="trakt-loading-box"><span class="spinner-small"></span> ${esc(t('trakt_loading'))}</div>`;
            const updated = await invoke('trakt_get_calendar_global', {
              startDate,
              forceRefresh: true,
            });
            const saved = await invoke('trakt_get_global_calendar_filters').catch(() => nextFilters);
            if (filterSeq !== viewLoadSeq || !traktModeActive) return;
            renderCalendarView(viewport, updated, 'global', {
              filters: saved,
              onChangeFilters: handleGlobalFilterChange,
            });
          } catch (err) {
            if (filterSeq !== viewLoadSeq || !traktModeActive) return;
            viewport.innerHTML = `<div class="trakt-error-box">${esc(formatTraktError(err))}</div>`;
          }
        };
        renderCalendarView(viewport, eps, 'global', {
          filters,
          onChangeFilters: handleGlobalFilterChange,
        });
      }
    } catch (err) {
      if (seq !== viewLoadSeq || !traktModeActive) return;
      viewport.innerHTML = `<div class="trakt-error-box">${esc(formatTraktError(err))}</div>`;
    }
  }

  async function enterTraktMode(subView = 'dashboard') {
    if (!authStatus.enabled) return;
    traktModeActive = true;
    currentSubView = subView;
    document.body.classList.add('trakt-mode');
    document.getElementById('epgViewport').classList.add('u-hidden');
    document.getElementById('traktViewport').classList.remove('u-hidden');
    document.getElementById('traktHeaderControls').classList.remove('u-hidden');
    updateHeaderVisibility();
    updateSyncStatusUi();
    await loadActiveTraktView();
  }

  function exitTraktMode(options = {}) {
    traktModeActive = false;
    viewLoadSeq++;
    document.body.classList.remove('trakt-mode');
    document.getElementById('traktViewport').classList.add('u-hidden');
    document.getElementById('epgViewport').classList.remove('u-hidden');
    document.getElementById('traktHeaderControls').classList.add('u-hidden');
    updateHeaderVisibility();
    if (activeDeviceFlow) {
      stopLoginFallbackTimer();
      activeDeviceFlow = null;
      invoke('trakt_cancel_login').catch(() => {});
    }
    if (!options.skipRender && typeof renderEPG === 'function') {
      renderEPG();
    }
  }

  // force = false: activity based sync (header button) - downloads only what changed on Trakt.
  // force = true:  full re-download of everything (Settings > Trakt.tv).
  async function triggerManualSync(force = false) {
    if (!authStatus.loggedIn || manualSyncInFlight) return;
    manualSyncInFlight = true;
    syncStatus.status = 'syncing';
    syncStatus.step = '';
    syncStatus.current = 0;
    syncStatus.total = 0;
    syncStatus.isFull = !!force;
    syncStatus.offline = false;
    updateSyncStatusUi();
    try {
      const res = await invoke('trakt_sync', { force: !!force });
      syncStatus.step = '';
      syncStatus.current = 0;
      syncStatus.total = 0;
      if (res && res.cancelled) {
        syncStatus.status = 'cancelled';
        syncStatus.offline = false;
        if (res.lastSyncOkAt) syncStatus.lastSyncOkAt = res.lastSyncOkAt;
        updateSyncStatusUi();
        if (traktModeActive) {
          await loadActiveTraktView();
        }
        if (document.getElementById('tabPaneTrakt')?.classList.contains('active')) {
          await loadSettingsTab({ preserveStatus: 'cancelled' });
        }
      } else {
        syncStatus.status = 'idle';
        syncStatus.offline = false;
        if (res && res.lastSyncOkAt) syncStatus.lastSyncOkAt = res.lastSyncOkAt;
        updateSyncStatusUi();
        if (res && res.throttled) {
          // A sync finished less than a minute ago: nothing was requested, the local data is current.
          showToast(t('trakt_sync_up_to_date'));
        } else {
          if (traktModeActive) {
            await loadActiveTraktView();
          }
          if (document.getElementById('tabPaneTrakt')?.classList.contains('active')) {
            await loadSettingsTab();
          }
        }
      }
    } catch (err) {
      syncStatus.step = '';
      syncStatus.current = 0;
      syncStatus.total = 0;
      syncStatus.status = String(err).includes('TRAKT_OFFLINE') ? 'offline' : 'error';
      syncStatus.offline = syncStatus.status === 'offline';
      updateSyncStatusUi();
      showTraktError(err);
    } finally {
      manualSyncInFlight = false;
    }
  }

  async function cancelManualSync() {
    if (syncStatus.status !== 'syncing') return;
    syncStatus.status = 'stopping';
    updateSyncStatusUi();
    try {
      await invoke('trakt_cancel_sync');
    } catch (err) {
      console.warn('[Trakt] trakt_cancel_sync error:', err);
    }
  }

  // ==========================================================================
  // EPG Grid Title Matching
  // ==========================================================================

  async function prepareMatchesForEpg(programmes) {
    watchlistTitleSet.clear();
    if (!authStatus.enabled || !programmes || programmes.length === 0) {
      updateWatchlistCountBadge();
      return;
    }
    try {
      const uniqueTitles = Array.from(new Set(programmes.map((p) => p.title).filter(Boolean)));
      if (uniqueTitles.length === 0) {
        updateWatchlistCountBadge();
        return;
      }
      const matched = await invoke('trakt_match_titles', { rawTitles: uniqueTitles });
      watchlistTitleSet = new Set(matched || []);
    } catch (e) {
      console.warn('[Trakt] prepareMatchesForEpg error:', e);
    }
    updateWatchlistCountBadge();
  }

  function isOnWatchlist(rawTitle) {
    if (!authStatus.enabled || !rawTitle) return false;
    return watchlistTitleSet.has(rawTitle);
  }

  // ==========================================================================
  // Modal Integration (EPG Modal + Trakt Card Modal)
  // ==========================================================================

  function detachFromModal() {
    modalAttachSeq++;
    const sec = document.getElementById('modalTraktSection');
    const badges = document.getElementById('modalTraktBadges');
    const posterMeta = document.getElementById('modalPosterTraktMeta');
    const wlBtn = document.getElementById('modalTraktWatchlistBtn');
    const wlStatus = document.getElementById('modalTraktWatchlistStatus');
    const netBadge = document.getElementById('modalTraktNetworkBadge');
    const drawer = document.getElementById('modalTraktDrawerSection');
    if (sec) sec.classList.add('u-hidden');
    if (badges) badges.innerHTML = '';
    if (posterMeta) posterMeta.classList.add('u-hidden');
    if (wlBtn) {
      wlBtn.classList.add('u-hidden');
      wlBtn.onclick = null;
    }
    if (wlStatus) {
      wlStatus.classList.add('u-hidden');
      wlStatus.textContent = '';
    }
    if (netBadge) {
      netBadge.classList.add('u-hidden');
      netBadge.textContent = '';
    }
    if (drawer) {
      drawer.classList.add('u-hidden');
      drawer.innerHTML = '';
    }
    if (typeof updateModalLeftColVisibility === 'function') {
      updateModalLeftColVisibility();
    }
  }

  function renderModalTraktSection(data, tmdbData) {
    const sec = document.getElementById('modalTraktSection');
    const badges = document.getElementById('modalTraktBadges');
    const posterMeta = document.getElementById('modalPosterTraktMeta');
    const wlBtn = document.getElementById('modalTraktWatchlistBtn');
    const wlStatus = document.getElementById('modalTraktWatchlistStatus');
    const netBadge = document.getElementById('modalTraktNetworkBadge');
    if (!sec || !badges || !wlBtn) return;

    if (!data) {
      sec.classList.add('u-hidden');
      if (posterMeta) posterMeta.classList.add('u-hidden');
      if (netBadge) {
        netBadge.classList.add('u-hidden');
        netBadge.textContent = '';
      }
      if (typeof updateModalLeftColVisibility === 'function') {
        updateModalLeftColVisibility();
      }
      return;
    }

    let badgesHtml = '';
    if (data.rating && data.rating > 0) {
      const votesStr = data.votes ? ` (${data.votes >= 1000 ? (data.votes / 1000).toFixed(1) + 'k' : data.votes})` : '';
      badgesHtml += `<span class="trakt-modal-badge trakt-badge-rating">
        <img src="images/trakt/logo.svg" alt="Trakt" class="trakt-badge-logo" />
        ${iconHtml('trakt-rating-filled', 12)} ${data.rating.toFixed(1)} / 10${esc(votesStr)}
      </span>`;
    }
    if (data.status) {
      badgesHtml += `<span class="trakt-modal-badge trakt-badge-status">${esc(data.status)}</span>`;
    }
    badges.innerHTML = badgesHtml;
    sec.classList.toggle('u-hidden', !badgesHtml);

    if (data.loggedIn) {
      wlBtn.classList.remove('u-hidden');
      const updateWlBtn = (onWl) => {
        data.onWatchlist = onWl;
        if (onWl) {
          wlBtn.className = 'btn trakt-modal-wl-btn is-on';
          wlBtn.innerHTML = `${iconHtml('trakt-watchlist-on', 16)} <span>${esc(t('trakt_watchlist_added'))}</span>`;
        } else {
          wlBtn.className = 'btn btn-blue trakt-modal-wl-btn';
          wlBtn.innerHTML = `${iconHtml('trakt-watchlist-add', 16)} <span>${esc(t('trakt_watchlist_add'))}</span>`;
        }
      };
      updateWlBtn(!!data.onWatchlist);

      wlBtn.onclick = async () => {
        wlBtn.disabled = true;
        if (wlStatus) wlStatus.classList.add('u-hidden');
        try {
          if (data.onWatchlist) {
            const queued = await invoke('trakt_watchlist_remove', {
              traktId: data.traktId,
              mediaType: data.mediaType,
            });
            updateWlBtn(false);
            if (queued && wlStatus) {
              wlStatus.textContent = t('trakt_error_offline');
              wlStatus.classList.remove('u-hidden');
            }
          } else {
            const queued = await invoke('trakt_watchlist_add', {
              traktId: data.traktId,
              mediaType: data.mediaType,
              title: data.title,
              year: data.year || null,
              tmdbId: data.tmdbId || null,
              posterPath: tmdbData?.posterPath || null,
              titleHu: tmdbData?.huTitle || null,
            });
            updateWlBtn(true);
            if (queued && wlStatus) {
              wlStatus.textContent = t('trakt_error_offline');
              wlStatus.classList.remove('u-hidden');
            }
          }
        } catch (err) {
          if (wlStatus) {
            wlStatus.textContent = formatTraktError(err);
            wlStatus.classList.remove('u-hidden');
          } else {
            showTraktError(err);
          }
        } finally {
          wlBtn.disabled = false;
        }
      };
    } else {
      wlBtn.classList.add('u-hidden');
    }

    const hasNetwork = !!(data.network && String(data.network).trim());
    if (netBadge) {
      if (hasNetwork) {
        netBadge.textContent = data.network;
        netBadge.title = data.network;
        netBadge.classList.remove('u-hidden');
      } else {
        netBadge.textContent = '';
        netBadge.classList.add('u-hidden');
      }
    }

    if (posterMeta) {
      posterMeta.classList.toggle('u-hidden', !(data.loggedIn || hasNetwork));
    }
    if (typeof updateModalLeftColVisibility === 'function') {
      updateModalLeftColVisibility();
    }
  }

  async function attachToModal({ title, tmdbId, mediaType, tmdbData }) {
    if (!authStatus.enabled || !title) return;
    const seq = ++modalAttachSeq;
    try {
      const data = await invoke('trakt_resolve_for_title', {
        title,
        tmdbId: tmdbId || null,
        mediaType: mediaType || null,
      });
      if (seq !== modalAttachSeq) return;
      if (!document.getElementById('modalOverlay')?.classList.contains('active')) return;
      renderModalTraktSection(data, tmdbData);

      if (data && data.traktId) {
        const drawer = document.getElementById('modalTraktDrawerSection');
        if (drawer && renderProgressAndAiringsInto) {
          const aliases = Array.from(
            new Set(
              [
                title,
                tmdbData?.huTitle,
                tmdbData?.enTitle,
                tmdbData?.originalTitle,
              ].filter(Boolean)
            )
          );
          renderProgressAndAiringsInto(
            drawer,
            {
              traktId: data.traktId,
              tmdbId: data.tmdbId || tmdbData?.tmdbId || null,
              mediaType: data.mediaType,
              title: data.title,
              aliases,
            },
            () => seq !== modalAttachSeq
          );
        }
      }
    } catch (e) {
      console.warn('[Trakt] attachToModal error:', e);
    }
  }

  async function openTraktItemModal(item) {
    if (typeof closeFixOverlay === 'function') closeFixOverlay();
    if (typeof resetModalTmdbUi === 'function') resetModalTmdbUi();
    detachFromModal();
    detachGoogleCalendarFromModal();

    const seq = ++modalAttachSeq;
    setModalContext(item.title, null, item.overview || '');

    document.getElementById('modalTitle').textContent = item.title;
    document.getElementById('modalTime').textContent = item.subtitle || (item.year ? String(item.year) : '');
    document.getElementById('modalDesc').textContent = item.overview || t('modal_no_desc');
    if (typeof updateModalFavStar === 'function') updateModalFavStar();

    const tmdbLoading = document.getElementById('modalTmdbLoading');
    if (tmdbLoading) tmdbLoading.classList.toggle('u-hidden', !item.tmdbId);
    document.getElementById('modalOverlay').classList.add('active');

    let tmdbData = null;
    if (item.tmdbId) {
      try {
        const lang = getCurrentLang() === 'english' ? 'en-US' : 'hu-HU';
        tmdbData = await invoke('get_tmdb_details_by_id', {
          tmdbId: item.tmdbId,
          mediaType: item.mediaType || 'show',
          lang,
        });
        if (seq !== modalAttachSeq) return;
        if (tmdbLoading) tmdbLoading.classList.add('u-hidden');
        if (tmdbData && typeof renderTmdbDetailsInModal === 'function') {
          renderTmdbDetailsInModal(tmdbData, { preserveTrakt: true, activeLang: lang });
        }
      } catch (_) {
        if (tmdbLoading) tmdbLoading.classList.add('u-hidden');
      }
    }

    if (seq !== modalAttachSeq) return;

    // Resolve Trakt metadata (rating, votes, status, watchlist state)
    let resolvedTrakt = null;
    try {
      resolvedTrakt = await invoke('trakt_resolve_for_title', {
        title: item.originalTitle || item.title,
        tmdbId: item.tmdbId || null,
        mediaType: item.mediaType === 'movie' ? 'movie' : 'tv',
      });
      if (seq !== modalAttachSeq) return;
      renderModalTraktSection(
        resolvedTrakt || {
          traktId: item.traktId,
          tmdbId: item.tmdbId,
          mediaType: item.mediaType || 'show',
          title: item.originalTitle || item.title,
          year: item.year,
          rating: item.rating,
          votes: item.votes,
          status: item.status,
          network: item.network,
          overview: item.overview,
          onWatchlist: !!item.onWatchlist,
          loggedIn: authStatus.loggedIn,
        },
        tmdbData
      );
    } catch (_) {}

    if (seq !== modalAttachSeq) return;

    // Render Season/Episode Progress Drawer + EPG Airings
    const drawerEl = document.getElementById('modalTraktDrawerSection');
    if (drawerEl) {
      const aliases = Array.from(
        new Set(
          [
            item.title,
            item.originalTitle,
            tmdbData?.huTitle,
            tmdbData?.enTitle,
            tmdbData?.originalTitle,
            ...(item.aliases || []),
          ].filter(Boolean)
        )
      );
      renderProgressAndAiringsInto(
        drawerEl,
        {
          ...item,
          traktId: item.traktId || resolvedTrakt?.traktId,
          tmdbId: item.tmdbId || tmdbData?.tmdbId || resolvedTrakt?.tmdbId || null,
          aliases,
        },
        () => seq !== modalAttachSeq
      );
    }
  }

  // ==========================================================================
  // Settings Tab
  // ==========================================================================

  async function loadSettingsTab(options = {}) {
    const pane = document.getElementById('tabPaneTrakt');
    if (!pane) return;

    try {
      authStatus = await invoke('trakt_get_auth_status');
      const freshSync = await invoke('trakt_get_sync_status');
      syncStatus = { ...syncStatus, ...freshSync };
      if (options.preserveStatus && syncStatus.status === 'idle') {
        syncStatus.status = options.preserveStatus;
      }
    } catch (e) {
      console.error('[Trakt] loadSettingsTab error:', e);
    }

    let tmdbKey = null;
    try {
      tmdbKey = await invoke('get_tmdb_api_key');
    } catch (_) {}

    let stats = null;
    if (authStatus.loggedIn) {
      try {
        stats = await invoke('trakt_get_stats');
      } catch (_) {}
    }

    const accountCard = document.getElementById('traktSettingsAccountCard');
    const statsCard = document.getElementById('traktSettingsStatsCard');
    const tmdbWarn = document.getElementById('traktTmdbMissingWarning');
    const masterCheck = document.getElementById('traktMasterSwitch');
    const customIdInput = document.getElementById('traktCustomClientIdInput');
    const customSecretInput = document.getElementById('traktCustomClientSecretInput');

    if (tmdbWarn) {
      tmdbWarn.classList.toggle('u-hidden', !!(tmdbKey && tmdbKey.trim()));
    }
    if (masterCheck) {
      masterCheck.checked = !!authStatus.enabled;
      masterCheck.onchange = async () => {
        await invoke('trakt_set_enabled', { enabled: masterCheck.checked });
        authStatus.enabled = masterCheck.checked;
        updateHeaderVisibility();
      };
    }
    if (customIdInput) {
      customIdInput.value = authStatus.customClientId || '';
    }
    if (customSecretInput) {
      customSecretInput.value = '';
      customSecretInput.placeholder = authStatus.hasCustomClientSecret
        ? t('trakt_custom_secret_saved_placeholder')
        : t('trakt_custom_secret_placeholder');
    }

    if (accountCard) {
      if (authStatus.loggedIn) {
        const prof = authStatus.profile || { username: 'Trakt User', vip: false };
        const locale = getCurrentLang() === 'english' ? 'en-US' : 'hu-HU';
        const joinedStr = prof.joinedAt ? new Date(prof.joinedAt).toLocaleDateString(locale) : '-';
        const syncStr = syncStatus.lastSyncOkAt
          ? new Date(syncStatus.lastSyncOkAt).toLocaleString(locale)
          : t('trakt_sync_never');
        const overdueClass = syncStatus.overdue ? 'is-overdue' : '';

        const avatarSrc =
          prof.avatarUrl && cachedAvatarUrl === prof.avatarUrl && cachedAvatarDataUri
            ? cachedAvatarDataUri
            : null;

        accountCard.innerHTML = `
          <div class="trakt-profile-row">
            ${avatarSrc ? `<img src="${esc(avatarSrc)}" alt="" class="trakt-profile-avatar" />` : `<div class="trakt-profile-avatar-ph" id="traktProfileAvatarPh">${iconHtml('trakt-login', 24)}</div>`}
            <div class="trakt-profile-info">
              <div class="trakt-profile-name">
                <strong>${esc(prof.name || prof.username)}</strong>
                <span class="trakt-profile-handle">@${esc(prof.username)}</span>
                ${prof.vip ? '<span class="trakt-vip-badge">VIP</span>' : ''}
              </div>
              <div class="trakt-profile-meta">${esc(t('trakt_member_since'))} ${esc(joinedStr)}</div>
              <div class="trakt-profile-sync ${overdueClass}" id="traktSettingsSyncStatus">
                ${esc(t('trakt_sync_last_ok'))} ${esc(syncStr)}
              </div>
              <div class="trakt-settings-sync-live u-hidden" id="traktSettingsSyncLive"></div>
            </div>
            <div class="trakt-profile-actions">
              <button type="button" class="btn btn-blue" id="traktSettingsSyncBtn" title="${esc(t('trakt_btn_full_sync_hint'))}">
                ${iconHtml('trakt-sync', 16)} <span>${esc(t('trakt_btn_full_sync'))}</span>
              </button>
              <button type="button" class="btn btn-red" id="traktSettingsLogoutBtn">
                ${iconHtml('trakt-logout', 16)} <span>${esc(t('trakt_btn_logout'))}</span>
              </button>
            </div>
          </div>
        `;

        if (prof.avatarUrl && !avatarSrc) {
          const avatarUrl = prof.avatarUrl;
          const phEl = accountCard.querySelector('#traktProfileAvatarPh');
          invoke('cache_logo', { url: avatarUrl, imageType: 'avatar' })
            .then((dataUri) => {
              if (!dataUri) return;
              cachedAvatarUrl = avatarUrl;
              cachedAvatarDataUri = dataUri;
              if (phEl && phEl.isConnected) {
                const img = document.createElement('img');
                img.className = 'trakt-profile-avatar';
                img.alt = '';
                img.src = dataUri;
                phEl.replaceWith(img);
              }
            })
            .catch(() => {});
        }

        const sBtn = accountCard.querySelector('#traktSettingsSyncBtn');
        if (sBtn) {
          sBtn.onclick = async () => {
            if (syncStatus.status === 'syncing') {
              await cancelManualSync();
              return;
            }
            if (syncStatus.status === 'stopping') return;
            const ok = await showConfirm(t('trakt_btn_full_sync'), t('trakt_confirm_full_sync'));
            if (!ok) return;
            triggerManualSync(true);
          };
        }

        const lBtn = accountCard.querySelector('#traktSettingsLogoutBtn');
        if (lBtn) {
          lBtn.onclick = async () => {
            const ok = await showConfirm(t('trakt_btn_logout'), t('trakt_confirm_logout'));
            if (!ok) return;
            await invoke('trakt_logout');
            authStatus.loggedIn = false;
            authStatus.profile = null;
            cachedAvatarUrl = null;
            cachedAvatarDataUri = null;
            watchlistTitleSet.clear();
            updateWatchlistCountBadge();
            await loadSettingsTab();
          };
        }
      } else {
        renderLoginCard(accountCard);
      }
    }

    if (statsCard) {
      if (authStatus.loggedIn && stats) {
        statsCard.classList.remove('u-hidden');
        statsCard.innerHTML = `
          <div class="trakt-cache-row">
            <span class="trakt-cache-info">
              ${esc(t('trakt_cache_records'))}: ${stats.cacheItemsCount} meta · ${stats.cacheCalendarCount} cal · ${stats.cacheHistoryCount} hist · ${stats.cacheEpisodesCount} ep
              ${stats.pendingActionsCount > 0 ? ` · ${stats.pendingActionsCount} pending` : ''}
            </span>
            <button type="button" class="btn btn-red" id="traktClearCacheBtn">
              ${iconHtml('trakt-clear-cache', 15)} <span>${esc(t('trakt_btn_clear_cache'))}</span>
            </button>
          </div>
        `;

        const clearBtn = statsCard.querySelector('#traktClearCacheBtn');
        if (clearBtn) {
          clearBtn.onclick = async () => {
            await invoke('trakt_clear_cache');
            await showAlert(t('alert_success_title'), t('trakt_cache_cleared'));
            await loadSettingsTab();
          };
        }
      } else {
        statsCard.classList.add('u-hidden');
      }
    }

    const saveKeysBtn = document.getElementById('traktSaveCustomKeysBtn');
    if (saveKeysBtn) {
      saveKeysBtn.onclick = async () => {
        const cid = customIdInput ? customIdInput.value.trim() : '';
        const csec = customSecretInput ? customSecretInput.value.trim() : '';
        await invoke('trakt_set_custom_keys', {
          clientId: cid,
          clientSecret: csec || null,
        });
        await showAlert(t('alert_success_title'), t('trakt_custom_keys_saved'));
        await loadSettingsTab();
      };
    }

    updateSyncStatusUi();
  }

  // ==========================================================================
  // Initialization & Event Listeners
  // ==========================================================================

  async function init() {
    if (Icons) {
      Icons.replace();
    }

    try {
      authStatus = await invoke('trakt_get_auth_status');
      syncStatus = { ...syncStatus, ...(await invoke('trakt_get_sync_status')) };
    } catch (e) {
      console.warn('[Trakt] init status error:', e);
    }

    updateHeaderVisibility();
    updateSyncStatusUi();

    const traktBtn = document.getElementById('traktBtn');
    if (traktBtn) traktBtn.onclick = () => enterTraktMode('dashboard');

    const backBtn = document.getElementById('traktBackBtn');
    if (backBtn) backBtn.onclick = () => exitTraktMode();

    const wlBtn = document.getElementById('traktWatchlistViewBtn');
    if (wlBtn) {
      wlBtn.onclick = () => {
        currentSubView = currentSubView === 'watchlist' ? 'dashboard' : 'watchlist';
        loadActiveTraktView();
      };
    }

    const wlEpgBtn = document.getElementById('traktWatchlistEpgBtn');
    if (wlEpgBtn) {
      wlEpgBtn.onclick = () => {
        currentSubView = currentSubView === 'watchlist_epg' ? 'dashboard' : 'watchlist_epg';
        loadActiveTraktView();
      };
    }

    const myCalBtn = document.getElementById('traktMyCalendarBtn');
    if (myCalBtn) {
      myCalBtn.onclick = () => {
        currentSubView = currentSubView === 'calendar_my' ? 'dashboard' : 'calendar_my';
        loadActiveTraktView();
      };
    }

    const globalCalBtn = document.getElementById('traktGlobalCalendarBtn');
    if (globalCalBtn) {
      globalCalBtn.onclick = () => {
        currentSubView = currentSubView === 'calendar_global' ? 'dashboard' : 'calendar_global';
        loadActiveTraktView();
      };
    }

    const syncBtn = document.getElementById('traktSyncBtn');
    if (syncBtn) syncBtn.onclick = () => triggerManualSync(false);

    // Tauri background events
    if (hasTauri()) {
      listen('trakt-login-state', async (event) => {
        const payload = event.payload || {};
        if (payload.status === 'ok') {
          await handleLoginSuccess(payload.profile || null);
        } else if (payload.status === 'logged_out') {
          stopLoginFallbackTimer();
          activeDeviceFlow = null;
          authStatus.loggedIn = false;
          authStatus.profile = null;
          cachedAvatarUrl = null;
          cachedAvatarDataUri = null;
          watchlistTitleSet.clear();
          updateWatchlistCountBadge();
          updateSyncStatusUi();
          if (traktModeActive) await loadActiveTraktView();
          if (document.getElementById('tabPaneTrakt')?.classList.contains('active')) {
            await loadSettingsTab();
          }
        } else if (payload.status === 'error') {
          stopLoginFallbackTimer();
          activeDeviceFlow = null;
          document.querySelectorAll('.trakt-login-error').forEach((errEl) => {
            errEl.textContent = payload.message || t('status_error');
            errEl.classList.remove('u-hidden');
          });
        }
      }).catch((e) => console.warn('[Trakt] listen trakt-login-state failed:', e));

      listen('trakt-sync-state', async (event) => {
        const payload = event.payload || {};
        const prevStatus = syncStatus.status;
        syncStatus.status = payload.status || 'idle';
        syncStatus.step = payload.step || '';
        syncStatus.current = Number(payload.current) || 0;
        syncStatus.total = Number(payload.total) || 0;
        if (typeof payload.isFull === 'boolean') {
          syncStatus.isFull = payload.isFull;
        }
        syncStatus.offline = payload.status === 'offline';
        if (payload.lastSyncOkAt) {
          syncStatus.lastSyncOkAt = payload.lastSyncOkAt;
        }
        updateSyncStatusUi();
        const finishedFromBusy =
          (prevStatus === 'syncing' || prevStatus === 'stopping') &&
          (syncStatus.status === 'idle' || syncStatus.status === 'cancelled');
        if (finishedFromBusy && !manualSyncInFlight) {
          if (traktModeActive) {
            await loadActiveTraktView();
          }
          if (document.getElementById('tabPaneTrakt')?.classList.contains('active')) {
            await loadSettingsTab(
              syncStatus.status === 'cancelled' ? { preserveStatus: 'cancelled' } : {}
            );
          }
        }
      }).catch((e) => console.warn('[Trakt] listen trakt-sync-state failed:', e));

      listen('trakt-watchlist-changed', async () => {
        if (state.epgData && state.epgData.programmes) {
          await prepareMatchesForEpg(state.epgData.programmes);
          if (!traktModeActive) {
            renderEPG();
          }
        } else {
          updateWatchlistCountBadge();
        }
        // Refresh the global calendar, watchlist, or watchlist-on-EPG view so watchlist state reflects the change
        if (traktModeActive && currentSubView === 'watchlist_epg') {
          await refreshWatchlistEpgIfActive();
        } else if (traktModeActive && (currentSubView === 'calendar_global' || currentSubView === 'watchlist')) {
          const scrollEl =
            document.getElementById('traktCalendarDays') ||
            document.getElementById('traktWatchlistGrid');
          const prevScroll = scrollEl ? scrollEl.scrollTop : 0;
          await loadActiveTraktView();
          const newScrollEl =
            document.getElementById('traktCalendarDays') ||
            document.getElementById('traktWatchlistGrid');
          if (newScrollEl && prevScroll > 0) {
            newScrollEl.scrollTop = prevScroll;
          }
        }
      }).catch((e) => console.warn('[Trakt] listen trakt-watchlist-changed failed:', e));
    }

    // Window focus sync (throttled to 15 min in Rust)
    window.addEventListener('focus', () => {
      if (authStatus.enabled && authStatus.loggedIn) {
        invoke('trakt_on_window_focus').catch(() => {});
      }
    });

    if (typeof addLanguageChangeListener === 'function') {
      addLanguageChangeListener(() => {
        updateSyncStatusUi();
        if (traktModeActive) loadActiveTraktView();
        if (document.getElementById('tabPaneTrakt')?.classList.contains('active')) {
          loadSettingsTab();
        }
      });
    }
  }

  async function refreshDashboardIfActive() {
    if (!traktModeActive || currentSubView !== 'dashboard') return;
    const viewport = document.getElementById('traktViewport');
    if (!viewport) return;
    const todayList = document.getElementById('traktTodayList');
    const prevScrollTop = todayList ? todayList.scrollTop : 0;
    try {
      const startDate = TraktDates.getLocalYesterdayYmd();
      const data = await invoke('trakt_get_dashboard_data', {
        startDate,
        forceRefresh: false,
      });
      if (!traktModeActive || currentSubView !== 'dashboard') return;
      renderDashboard(viewport, data, { preservePage: true });
      const newTodayList = document.getElementById('traktTodayList');
      if (newTodayList && prevScrollTop > 0) {
        newTodayList.scrollTop = prevScrollTop;
      }
    } catch (_) {}
  }

  function isTraktMode() {
    return traktModeActive;
  }

export {
  init as initTraktUI,
  enterTraktMode,
  exitTraktMode,
  prepareMatchesForEpg,
  isOnWatchlist,
  attachToModal,
  detachFromModal,
  openTraktItemModal,
  loadSettingsTab,
  refreshDashboardIfActive,
  showTraktError,
  showToast,
  isTraktMode,
};
