const { invoke } = window.__TAURI__.core;

let epgData = { channels: [], programmes: [] };
let selectedDateStr = "";
let currentSourceId = "";
let currentStepMin = 20;
let currentMinuteHeight = 2;
let currentLayout = "vertical"; // "vertical" | "horizontal"
let activeSingleChannelId = null;
let currentModalTitle = "";
let currentModalDesc = "";
let searchTimeout = null;

let sources = [];
let favorites = [];
let settings = {};
const downloadingSources = new Set();
let isUpdatingAllSources = false;

const LOCALE_COMPARE_OPTIONS = Object.freeze({
  sensitivity: "base",
  numeric: true,
});

const VIEW_HEIGHT_MAP = {
  60: 1.2,
  30: 1.5,
  20: 2,
  10: 3.5,
  5: 6,
};

// In horizontal mode, timeline cells need enough width so "00:00" fits comfortably
const HORIZONTAL_SCALE_MAP = {
  60: 3.0, // 60 min -> 180px cell
  30: 3.6, // 30 min -> 108px cell
  20: 4.5, // 20 min -> 90px cell
  10: 7.0, // 10 min -> 70px cell
  5: 12.0, // 5 min -> 60px cell
};
