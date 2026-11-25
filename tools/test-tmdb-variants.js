const assert = require("assert");
const fs = require("fs");
const path = require("path");
const vm = require("vm");

function makeMockElement(id) {
  const classes = new Set();
  const children = [];
  return {
    id,
    style: {},
    dataset: {},
    textContent: "",
    innerHTML: "",
    src: "",
    value: "",
    onclick: null,
    oninput: null,
    onkeydown: null,
    focus: () => {},
    select: () => {},
    children,
    classList: {
      add: (...cls) => cls.forEach((c) => classes.add(c)),
      remove: (...cls) => cls.forEach((c) => classes.delete(c)),
      contains: (c) => classes.has(c),
    },
    addEventListener: () => {},
    appendChild(child) {
      const idx = children.indexOf(child);
      if (idx !== -1) children.splice(idx, 1);
      children.push(child);
      return child;
    },
  };
}

function loadTmdbModule(initialLang = "hungarian") {
  const elements = new Map();
  const getEl = (id) => {
    if (!elements.has(id)) elements.set(id, makeMockElement(id));
    return elements.get(id);
  };

  const invokeCalls = [];
  const sandbox = {
    currentLang: initialLang,
    currentModalTitle: "MacGyver",
    currentModalDesc: "",
    currentSourceId: "src1",
    sources: [{ id: "src1", countryCode: "AT" }],
    window: {},
    navigator: {},
    document: {
      getElementById: getEl,
      querySelector: () => null,
      createElement: (tag) => makeMockElement(tag),
    },
    t: (key) =>
      ({
        badge_hu: "Magyar:",
        badge_en: "Angol:",
        badge_orig: "Eredeti:",
        media_type_tv: "Sorozat",
        media_type_movie: "Film",
        modal_no_desc: "Nincs leírás",
      })[key] || key,
    iconHtml: (name) => `<svg data-icon="${name}"></svg>`,
    escapeHtml: (s) => String(s),
    invoke: async (cmd, args) => {
      invokeCalls.push({ cmd, args });
      if (cmd === "get_tmdb_details_for_lang") {
        return {
          tmdbId: args.tmdbId,
          mediaType: args.mediaType,
          overview: `Overview in ${args.lang}`,
        };
      }
      return null;
    },
    requestAnimationFrame: (fn) => fn(),
    setTimeout,
    clearTimeout,
    console,
  };

  const tmdbSrc = fs.readFileSync(
    path.join(__dirname, "../public/javascript/modules/tmdb.js"),
    "utf8",
  );
  vm.runInNewContext(tmdbSrc, sandbox, { filename: "tmdb.js" });
  return { sandbox, getEl, invokeCalls };
}

function testMacGyverHungarianApp() {
  const { sandbox, getEl } = loadTmdbModule("hungarian");
  const tmdbData = {
    tmdbId: 67133,
    mediaType: "tv",
    title: "MacGyver",
    huTitle: "MacGyver",
    enTitle: "MacGyver",
    originalTitle: "MacGyver",
    originalLanguage: "en",
    overview: "Magyar leírás",
  };

  const variants = sandbox.getModalTitleVariants(tmdbData, "hungarian");
  assert.deepStrictEqual(
    Array.from(variants, (v) => [v.slot, v.tmdbLang, v.title]),
    [
      ["hu", "hu-HU", "MacGyver"],
      ["en", "en-US", "MacGyver"],
      ["orig", "en-US", "MacGyver"],
    ],
  );

  sandbox.renderTmdbDetailsInModal(tmdbData, { activeLang: "hu-HU" });
  const row = getEl("modalTitleVariants");
  assert.strictEqual(row.style.display, "flex");
  assert.deepStrictEqual(
    row.children.map((el) => el.id),
    ["modalHuTitleBadge", "modalEnTitleBadge", "modalOrigTitleBadge"],
  );
  assert.strictEqual(getEl("modalHuTitleBadge").style.display, "inline-flex");
  assert.strictEqual(getEl("modalEnTitleBadge").style.display, "inline-flex");
  assert.strictEqual(getEl("modalOrigTitleBadge").style.display, "inline-flex");
  assert.strictEqual(getEl("modalHuTitleBadge").classList.contains("active"), true);
  assert.strictEqual(getEl("modalEnTitleBadge").classList.contains("active"), false);
}

async function testSameTitleDifferentOriginalLangSwitch() {
  const { sandbox, getEl, invokeCalls } = loadTmdbModule("hungarian");
  const tmdbData = {
    tmdbId: 70523,
    mediaType: "tv",
    title: "Dark",
    huTitle: "Dark",
    enTitle: "Dark",
    originalTitle: "Dark",
    originalLanguage: "de",
    overview: "Magyar leírás",
  };

  sandbox.renderTmdbDetailsInModal(tmdbData, { activeLang: "hu-HU" });
  await getEl("modalEnTitleBadge").onclick();
  assert.strictEqual(
    getEl("modalTmdbOverviewText").textContent,
    "Overview in en-US",
  );
  assert.strictEqual(getEl("modalEnTitleBadge").classList.contains("active"), true);
  assert.strictEqual(getEl("modalHuTitleBadge").classList.contains("active"), false);

  await getEl("modalOrigTitleBadge").onclick();
  assert.strictEqual(
    getEl("modalTmdbOverviewText").textContent,
    "Overview in de-DE",
  );
  assert.strictEqual(getEl("modalOrigTitleBadge").classList.contains("active"), true);
  assert.strictEqual(invokeCalls[invokeCalls.length - 1].args.lang, "de-DE");
}

function testMissingHuTitleAndMissingOriginalTitle() {
  const { sandbox } = loadTmdbModule("hungarian");

  // German-fetched show where /translations had no HU title override
  const noHu = sandbox.getModalTitleVariants(
    {
      title: "Stirb langsam",
      huTitle: null,
      enTitle: "Die Hard",
      originalTitle: "Die Hard",
      originalLanguage: "en",
    },
    "hungarian",
  );
  assert.deepStrictEqual(
    Array.from(noHu, (v) => [v.slot, v.tmdbLang, v.title]),
    [
      ["hu", "hu-HU", "Die Hard"],
      ["en", "en-US", "Die Hard"],
      ["orig", "en-US", "Die Hard"],
    ],
  );

  // Show without originalTitle -> only app lang + English
  const noOrig = sandbox.getModalTitleVariants(
    {
      title: "Híradó",
      huTitle: "Híradó",
      enTitle: "News",
      originalTitle: null,
      originalLanguage: null,
    },
    "hungarian",
  );
  assert.deepStrictEqual(
    Array.from(noOrig, (v) => [v.slot, v.tmdbLang, v.title]),
    [
      ["hu", "hu-HU", "Híradó"],
      ["en", "en-US", "News"],
    ],
  );
}

function testEnglishAppLanguage() {
  const { sandbox, getEl } = loadTmdbModule("english");
  const tmdbData = {
    tmdbId: 71446,
    mediaType: "tv",
    title: "Money Heist",
    huTitle: "A nagy pénzrablás",
    enTitle: "Money Heist",
    originalTitle: "La casa de papel",
    originalLanguage: "es",
    overview: "English overview",
  };

  const variants = sandbox.getModalTitleVariants(tmdbData, "english");
  assert.deepStrictEqual(
    Array.from(variants, (v) => [v.slot, v.tmdbLang, v.title]),
    [
      ["en", "en-US", "Money Heist"],
      ["orig", "es-ES", "La casa de papel"],
    ],
  );

  sandbox.renderTmdbDetailsInModal(tmdbData, { activeLang: "en-US" });
  assert.strictEqual(getEl("modalHuTitleBadge").style.display, "none");
  assert.strictEqual(getEl("modalEnTitleBadge").style.display, "inline-flex");
  assert.strictEqual(getEl("modalOrigTitleBadge").style.display, "inline-flex");
  assert.strictEqual(getEl("modalEnTitleBadge").classList.contains("active"), true);
}

function testFixOverlayShowsOriginalProgramContext() {
  const { sandbox, getEl } = loadTmdbModule("hungarian");
  sandbox.currentModalTitle = "MacGyver";
  sandbox.currentModalDesc = "Angus MacGyver zseniális trükkökkel old meg veszélyes küldetéseket.";

  sandbox.openFixOverlay();
  assert.strictEqual(getEl("modalFixProgramContext").style.display, "block");
  assert.strictEqual(getEl("modalFixProgramTitle").textContent, "MacGyver");
  assert.strictEqual(
    getEl("modalFixProgramDesc").textContent,
    "Angus MacGyver zseniális trükkökkel old meg veszélyes küldetéseket.",
  );
  assert.strictEqual(getEl("modalFixProgramDesc").classList.contains("is-empty"), false);

  // Empty description falls back to localized modal_no_desc with muted style
  sandbox.currentModalDesc = "   ";
  sandbox.openFixOverlay();
  assert.strictEqual(getEl("modalFixProgramDesc").textContent, "Nincs leírás");
  assert.strictEqual(getEl("modalFixProgramDesc").classList.contains("is-empty"), true);
}

(async () => {
  testMacGyverHungarianApp();
  await testSameTitleDifferentOriginalLangSwitch();
  testMissingHuTitleAndMissingOriginalTitle();
  testEnglishAppLanguage();
  testFixOverlayShowsOriginalProgramContext();
  console.log("All TMDB title variant tests passed!");
})();
