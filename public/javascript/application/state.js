let customInvokeForTesting = null;

export function invoke(cmd, args) {
  if (customInvokeForTesting) {
    return customInvokeForTesting(cmd, args);
  }
  const fn = globalThis.window?.__TAURI__?.core?.invoke;
  if (typeof fn !== "function") {
    throw new Error(`Tauri invoke not available for command: ${cmd}`);
  }
  return fn(cmd, args);
}

export function listen(event, handler) {
  const fn = globalThis.window?.__TAURI__?.event?.listen;
  if (typeof fn !== "function") return Promise.resolve(() => {});
  return fn(event, handler);
}

export function hasTauri() {
  return typeof globalThis.window?.__TAURI__?.core?.invoke === "function";
}

export function setWindowTitle(title) {
  const getCurrentWindow = globalThis.window?.__TAURI__?.window?.getCurrentWindow;
  if (typeof getCurrentWindow !== "function") return Promise.resolve();
  try {
    const win = getCurrentWindow();
    if (win && typeof win.setTitle === "function") {
      return Promise.resolve(win.setTitle(title));
    }
  } catch {
    // Ignore when running outside a Tauri webview window
  }
  return Promise.resolve();
}

export function setInvokeForTesting(fn) {
  customInvokeForTesting = fn;
}

export const state = {
  epgData: { channels: [], programmes: [] },
  selectedDateStr: "",
  currentSourceId: "",
  currentStepMin: 20,
  currentMinuteHeight: 2,
  currentLayout: "vertical", // "vertical" | "horizontal"
  activeSingleChannelId: null,
  currentModalTitle: "",
  currentModalDesc: "",
  currentModalChannel: null,
  currentModalFetchIndex: 0,
  searchTimeout: null,

  sources: [],
  favorites: [],
  favoritesSet: new Set(),
  settings: {},
  downloadingSources: new Set(),
  isUpdatingAllSources: false,
  settingsEpgDirty: false,
};

export function setFavoritesList(list) {
  state.favorites = Array.isArray(list) ? list : [];
  state.favoritesSet = new Set(state.favorites);
}

export const LOCALE_COMPARE_OPTIONS = Object.freeze({
  sensitivity: "base",
  numeric: true,
});

export const VIEW_HEIGHT_MAP = {
  60: 1.2,
  30: 1.5,
  20: 2,
  10: 3.5,
  5: 6,
};

// In horizontal mode, timeline cells need enough width so "00:00" fits comfortably
export const HORIZONTAL_SCALE_MAP = {
  60: 3.0, // 60 min -> 180px cell
  30: 3.6, // 30 min -> 108px cell
  20: 4.5, // 20 min -> 90px cell
  10: 7.0, // 10 min -> 70px cell
  5: 12.0, // 5 min -> 60px cell
};
