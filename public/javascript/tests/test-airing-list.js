import assert from "node:assert";
import { setLanguageForTesting } from "../application/i18n.js";
import {
  normalizeSortLevels,
  parseSortLevelsSetting,
  sortAirings,
  groupAirings,
  filterAiringsByQuery,
  createMultiSortControls,
  createLiveSearchBox,
  createCollapseAllButton,
  createCollapsibleGroupElement,
  getGroupCollapseSummary,
  setAllGroupsCollapsed,
} from "../application/airing-list.js";

function makeMockElement(tag) {
  const classes = new Set();
  const attrs = new Map();
  const children = [];
  let _value = "";
  let _className = "";
  const el = {
    tagName: tag.toUpperCase(),
    id: "",
    type: "",
    dataset: {},
    get className() {
      return _className;
    },
    set className(v) {
      _className = String(v);
      classes.clear();
      _className
        .split(/\s+/)
        .filter(Boolean)
        .forEach((c) => classes.add(c));
    },
    textContent: "",
    innerHTML: "",
    placeholder: "",
    title: "",
    autocomplete: "",
    disabled: false,
    htmlFor: "",
    children,
    get value() {
      return _value;
    },
    set value(v) {
      _value = String(v);
    },
    get options() {
      return children;
    },
    setAttribute(k, v) {
      attrs.set(k, String(v));
    },
    getAttribute(k) {
      return attrs.get(k) ?? null;
    },
    appendChild(child) {
      children.push(child);
      return child;
    },
    querySelector(sel) {
      const cls = sel.startsWith(".") ? sel.slice(1) : null;
      if (!cls) return null;
      for (const c of children) {
        if (c.classList?.contains(cls)) return c;
        const nested = c.querySelector?.(sel);
        if (nested) return nested;
      }
      return null;
    },
    querySelectorAll(sel) {
      const cls = sel.startsWith(".") ? sel.slice(1) : null;
      if (!cls) return [];
      const out = [];
      for (const c of children) {
        if (c.classList?.contains(cls)) out.push(c);
        if (c.querySelectorAll) out.push(...c.querySelectorAll(sel));
      }
      return out;
    },
    classList: {
      add: (...cls) => cls.forEach((c) => classes.add(c)),
      remove: (...cls) => cls.forEach((c) => classes.delete(c)),
      toggle: (c, force) => {
        const next = force === undefined ? !classes.has(c) : !!force;
        if (next) classes.add(c);
        else classes.delete(c);
        return next;
      },
      contains: (c) => classes.has(c),
    },
    onclick: null,
    onchange: null,
    oninput: null,
    onkeydown: null,
    focus: () => {},
  };
  return el;
}

function setupTestEnv() {
  setLanguageForTesting("hungarian", {
    days: [
      "Vasárnap",
      "Hétfő",
      "Kedd",
      "Szerda",
      "Csütörtök",
      "Péntek",
      "Szombat",
    ],
    months: [
      "január",
      "február",
      "március",
      "április",
      "május",
      "június",
      "július",
      "augusztus",
      "szeptember",
      "október",
      "november",
      "december",
    ],
    day_today: "Ma",
    day_yesterday: "Tegnap",
    day_tomorrow: "Holnap",
    search_placeholder: "Keresés...",
    search_btn: "KERESÉS",
    clear_search: "Keresés törlése",
    sort_level_1: "1. rendezés:",
    sort_level_2: "2. rendezés:",
    sort_level_3: "3. rendezés:",
    sort_none: "-- Nincs --",
    sort_by_date: "Dátum",
    sort_by_title: "A-Z",
    sort_by_channel: "Csatorna szerint",
    sort_by_source: "Forrás szerint",
    airing_collapse_all: "Mind becsukása",
    airing_expand_all: "Mind kinyitása",
  });

  globalThis.document = {
    createElement: (tag) => makeMockElement(tag),
  };
}

function testNormalizeAndParseSortLevels() {
  assert.deepStrictEqual(normalizeSortLevels([], "date"), ["date"]);
  assert.deepStrictEqual(normalizeSortLevels(["invalid"], "title"), ["title"]);
  assert.deepStrictEqual(
    normalizeSortLevels(["title", "title", "date", "channel", "source_channel"]),
    ["title", "date", "channel"],
  );
  assert.deepStrictEqual(
    parseSortLevelsSetting('["source","channel","date"]', "title"),
    ["source", "channel", "date"],
  );
  assert.deepStrictEqual(
    parseSortLevelsSetting('["source_channel","date"]', "title"),
    ["source", "date"],
  );
  assert.deepStrictEqual(parseSortLevelsSetting("not-json", "title"), ["title"]);
}

function testMultiLevelSorting() {
  const items = [
    {
      id: 1,
      title: "Beta",
      channelName: "RTL",
      sourceName: "Forrás B",
      sourceId: "sb",
      channelId: "c1",
      startDate: new Date("2026-11-27T20:00:00"),
    },
    {
      id: 2,
      title: "Alpha",
      channelName: "M1",
      sourceName: "Forrás B",
      sourceId: "sb",
      channelId: "c2",
      startDate: new Date("2026-11-28T18:00:00"),
    },
    {
      id: 3,
      title: "Alpha",
      channelName: "HBO",
      sourceName: "Forrás A",
      sourceId: "sa",
      channelId: "c3",
      startDate: new Date("2026-11-26T21:00:00"),
    },
    {
      id: 4,
      title: "Alpha",
      channelName: "HBO",
      sourceName: "Forrás A",
      sourceId: "sa",
      channelId: "c3",
      startDate: new Date("2026-11-26T19:00:00"),
    },
  ];

  // Date first
  assert.deepStrictEqual(
    sortAirings(items, ["date"], "hungarian").map((x) => x.id),
    [4, 3, 1, 2],
  );

  // A-Z then Date
  assert.deepStrictEqual(
    sortAirings(items, ["title", "date"], "hungarian").map((x) => x.id),
    [4, 3, 2, 1],
  );

  // Channel then Date
  assert.deepStrictEqual(
    sortAirings(items, ["channel", "date"], "hungarian").map((x) => x.id),
    [4, 3, 2, 1],
  );

  // Source then Channel then Date
  assert.deepStrictEqual(
    sortAirings(items, ["source", "channel", "date"], "hungarian").map(
      (x) => x.id,
    ),
    [4, 3, 2, 1],
  );

  // Source then Date then Title (ignores channel when level 2 is date)
  const srcDateItems = [
    {
      id: 20,
      title: "Beta",
      channelName: "AlphaCh",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-27T20:00:00"),
    },
    {
      id: 21,
      title: "Alpha",
      channelName: "ZetaCh",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-26T20:00:00"),
    },
  ];
  assert.deepStrictEqual(
    sortAirings(srcDateItems, ["source", "date", "title"], "hungarian").map(
      (x) => x.id,
    ),
    [21, 20],
  );

  // Same Source, Channel and Date -> level 3 Title breaks tie
  const tieItems = [
    {
      id: 10,
      title: "Zebra",
      channelName: "HBO",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-26T20:00:00"),
    },
    {
      id: 11,
      title: "Alma",
      channelName: "HBO",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-26T20:00:00"),
    },
    {
      id: 12,
      title: "Körte",
      channelName: "HBO",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-26T19:00:00"),
    },
  ];
  assert.deepStrictEqual(
    sortAirings(tieItems, ["source", "date", "title"], "hungarian").map(
      (x) => x.id,
    ),
    [11, 12, 10],
  );

  // 1. Date, 2. A-Z (same calendar day, different start times -> must sort A-Z within the day!)
  const sameDayItems = [
    {
      id: 30,
      title: "Zebra",
      channelName: "RTL",
      sourceName: "Forrás B",
      startDate: new Date("2026-11-26T18:00:00"),
    },
    {
      id: 31,
      title: "Alma",
      channelName: "M1",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-26T21:00:00"),
    },
    {
      id: 32,
      title: "Béla",
      channelName: "HBO",
      sourceName: "Forrás C",
      startDate: new Date("2026-11-26T19:30:00"),
    },
    {
      id: 33,
      title: "Alma",
      channelName: "HBO",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-27T09:00:00"),
    },
  ];
  assert.deepStrictEqual(
    sortAirings(sameDayItems, ["date"], "hungarian").map((x) => x.id),
    [30, 32, 31, 33],
  );
  assert.deepStrictEqual(
    sortAirings(sameDayItems, ["date", "title"], "hungarian").map((x) => x.id),
    [31, 32, 30, 33],
  );
  assert.deepStrictEqual(
    sortAirings(sameDayItems, ["date", "channel"], "hungarian").map(
      (x) => x.id,
    ),
    [32, 31, 30, 33],
  );
  assert.deepStrictEqual(
    sortAirings(sameDayItems, ["date", "source"], "hungarian").map((x) => x.id),
    [31, 30, 32, 33],
  );

  // 1. A-Z, 2. Date, 3. Channel (same title & same calendar day, different times -> level 3 Channel wins!)
  const threeLevelItems = [
    {
      id: 40,
      title: "Dűne",
      channelName: "RTL",
      sourceName: "Forrás B",
      startDate: new Date("2026-11-26T18:00:00"),
    },
    {
      id: 41,
      title: "Dűne",
      channelName: "HBO",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-26T21:00:00"),
    },
    {
      id: 42,
      title: "Dűne",
      channelName: "HBO",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-27T10:00:00"),
    },
  ];
  assert.deepStrictEqual(
    sortAirings(threeLevelItems, ["title", "date", "channel"], "hungarian").map(
      (x) => x.id,
    ),
    [41, 40, 42],
  );
  assert.deepStrictEqual(
    sortAirings(threeLevelItems, ["title", "date"], "hungarian").map(
      (x) => x.id,
    ),
    [40, 41, 42],
  );
}

function testAll40SortPermutations() {
  const keys = ["date", "title", "channel", "source"];
  const combos = [];
  for (const k1 of keys) {
    combos.push([k1]);
    for (const k2 of keys) {
      if (k2 === k1) continue;
      combos.push([k1, k2]);
      for (const k3 of keys) {
        if (k3 === k1 || k3 === k2) continue;
        combos.push([k1, k2, k3]);
      }
    }
  }
  assert.strictEqual(combos.length, 40);

  // For every 2- and 3-level combo [k1, k2, (k3)], construct two items that are EQUAL on k1 (and k2)
  // even though their timestamps differ, and verify the final chosen level decides the order!
  for (const combo of combos) {
    const baseA = {
      id: "A",
      title: "Alma",
      channelName: "HBO",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-26T22:00:00"),
    };
    const baseB = {
      id: "B",
      title: "Alma",
      channelName: "HBO",
      sourceName: "Forrás A",
      startDate: new Date("2026-11-26T18:00:00"),
    };

    const lastKey = combo[combo.length - 1];
    if (lastKey === "date") {
      baseA.startDate = new Date("2026-11-26T22:00:00");
      baseB.startDate = new Date("2026-11-27T08:00:00");
    } else if (lastKey === "title") {
      baseA.title = "Alma";
      baseB.title = "Zebra";
    } else if (lastKey === "channel") {
      baseA.channelName = "HBO";
      baseB.channelName = "RTL";
    } else if (lastKey === "source") {
      baseA.sourceName = "Forrás A";
      baseB.sourceName = "Forrás B";
    }

    const sorted = sortAirings([baseB, baseA], combo, "hungarian");
    assert.strictEqual(
      sorted[0].id,
      "A",
      `Expected A before B for combo [${combo.join(", ")}]`,
    );
  }
}

function testDynamicGrouping() {
  const items = [
    {
      title: "Álom",
      channelName: "M1",
      sourceName: "S1",
      sourceId: "s1",
      channelId: "c1",
      startDate: new Date("2026-11-26T20:00:00"),
    },
    {
      title: "Beta",
      channelName: "M1",
      sourceName: "S2",
      sourceId: "s2",
      channelId: "c1",
      startDate: new Date("2026-11-26T21:00:00"),
    },
  ];

  const byTitle = groupAirings(sortAirings(items, ["title"]), "title");
  assert.deepStrictEqual(
    byTitle.map((g) => g.label),
    ["A", "B"],
  );

  const byChannel = groupAirings(sortAirings(items, ["channel"]), "channel");
  assert.deepStrictEqual(
    byChannel.map((g) => [g.label, g.items.length]),
    [["M1", 2]],
  );

  const bySource = groupAirings(
    sortAirings(items, ["source"]),
    "source",
  );
  assert.deepStrictEqual(
    bySource.map((g) => g.label),
    ["S1", "S2"],
  );

  const byDate = groupAirings(sortAirings(items, ["date"]), "date");
  assert.strictEqual(byDate.length, 1);
  assert.ok(byDate[0].key === "date:2026-11-26");
}

function testFilterAiringsByQuery() {
  const items = [
    {
      title: "Híradó",
      channelName: "M1 HD",
      sourceName: "Magyar EPG",
    },
    {
      title: "Különös dolgok S01E01",
      watchlistTitle: "Stranger Things",
      aliases: ["Stranger Things"],
      channelName: "RTL",
      sourceName: "Magyar EPG",
    },
    {
      title: "Dűne",
      channelName: "HBO",
      sourceName: "Angol Forrás",
    },
  ];

  assert.deepStrictEqual(
    filterAiringsByQuery(items, "hirado").map((x) => x.title),
    ["Híradó"],
  );
  assert.deepStrictEqual(
    filterAiringsByQuery(items, "stranger").map((x) => x.title),
    ["Különös dolgok S01E01"],
  );
  assert.deepStrictEqual(
    filterAiringsByQuery(items, "angol").map((x) => x.title),
    ["Dűne"],
  );
  assert.deepStrictEqual(
    filterAiringsByQuery(items, "kulonos dolgoo").map((x) => x.title),
    ["Különös dolgok S01E01"],
  );
}

function testMultiSortControlsCollisionHandling() {
  const changes = [];
  const ctrl = createMultiSortControls({
    initialLevels: ["date", "title", "channel"],
    defaultKey: "date",
    onChange: (lvls) => changes.push(lvls),
  });

  const selects = ctrl.element.children.map((g) => g.children[1]);
  assert.strictEqual(selects[0].value, "date");
  assert.strictEqual(selects[1].value, "title");
  assert.strictEqual(selects[2].value, "channel");
  assert.strictEqual(selects[2].disabled, false);

  // Changing level 1 to "title" collides with level 2 -> promotes level 3 ("channel") to level 2
  selects[0].value = "title";
  selects[0].onchange();
  assert.deepStrictEqual(ctrl.getLevels(), ["title", "channel"]);
  assert.strictEqual(selects[1].value, "channel");
  assert.strictEqual(selects[2].value, "");

  // Clearing level 2 disables and clears level 3
  selects[1].value = "";
  selects[1].onchange();
  assert.deepStrictEqual(ctrl.getLevels(), ["title"]);
  assert.strictEqual(selects[2].disabled, true);
  assert.strictEqual(changes.length, 2);
}

async function testLiveSearchBoxBehavior() {
  const calls = [];
  const box = createLiveSearchBox({
    inputId: "testSearchInput",
    clearBtnId: "testClearBtn",
    onFilter: (q) => calls.push(q),
  });

  const inputWrap = box.element.children[0];
  assert.strictEqual(box.element.children.length, 1, "No external Search/Trash buttons");
  const [input, clearBtn] = inputWrap.children;

  // Clear button starts hidden when input is empty
  assert.strictEqual(clearBtn.classList.contains("u-hidden"), true);

  // Debounced oninput (300ms) + clear button becomes visible
  input.value = "dune";
  input.oninput();
  assert.strictEqual(clearBtn.classList.contains("u-hidden"), false);
  assert.strictEqual(calls.length, 0, "oninput must debounce for 300ms");
  await new Promise((r) => setTimeout(r, 340));
  assert.deepStrictEqual(calls, ["dune"]);

  // Immediate Enter
  input.value = "fargo";
  input.onkeydown({ key: "Enter", preventDefault: () => {} });
  assert.deepStrictEqual(calls, ["dune", "fargo"]);

  // Escape when non-empty clears input, hides clearBtn, stops propagation, and triggers ""
  let stopped = false;
  input.onkeydown({
    key: "Escape",
    preventDefault: () => {},
    stopPropagation: () => {
      stopped = true;
    },
  });
  assert.strictEqual(stopped, true);
  assert.strictEqual(input.value, "");
  assert.strictEqual(clearBtn.classList.contains("u-hidden"), true);
  assert.deepStrictEqual(calls, ["dune", "fargo", ""]);

  // Escape when already empty does NOT stop propagation or trigger onFilter
  let stoppedEmpty = false;
  input.onkeydown({
    key: "Escape",
    preventDefault: () => {},
    stopPropagation: () => {
      stoppedEmpty = true;
    },
  });
  assert.strictEqual(stoppedEmpty, false);
  assert.deepStrictEqual(calls, ["dune", "fargo", ""]);

  // Inline clear button resets input, hides itself, and triggers ""
  input.value = "tulsa";
  input.oninput();
  assert.strictEqual(clearBtn.classList.contains("u-hidden"), false);
  clearBtn.onclick({ stopPropagation: () => {} });
  assert.strictEqual(input.value, "");
  assert.strictEqual(clearBtn.classList.contains("u-hidden"), true);
  assert.deepStrictEqual(calls, ["dune", "fargo", "", ""]);
  box.destroy();
}

function testCollapsibleGroupsAndFoldButton() {
  const container = makeMockElement("div");
  const collapsedKeys = new Set();

  const g1 = createCollapsibleGroupElement({
    groupKey: "date:2026-11-26",
    label: "Ma",
    count: 2,
    collapsedKeys,
  });
  const g2 = createCollapsibleGroupElement({
    groupKey: "date:2026-11-27",
    label: "Holnap",
    count: 1,
    collapsedKeys,
  });
  container.appendChild(g1.groupDiv);
  container.appendChild(g2.groupDiv);

  const foldCtrl = createCollapseAllButton({
    onToggle: (allCollapsed) => {
      setAllGroupsCollapsed(container, collapsedKeys, !allCollapsed);
      const summary = getGroupCollapseSummary(container, collapsedKeys);
      foldCtrl.updateState(summary.allCollapsed, summary.hasGroups);
    },
  });

  let summary = getGroupCollapseSummary(container, collapsedKeys);
  foldCtrl.updateState(summary.allCollapsed, summary.hasGroups);
  assert.strictEqual(foldCtrl.element.disabled, false);
  assert.strictEqual(foldCtrl.element.title, "Mind becsukása");
  assert.ok(foldCtrl.element.innerHTML.includes("tabler-icon-fold-up"));

  // Collapse all via button -> switches to fold-down + "Mind kinyitása"
  foldCtrl.element.onclick();
  assert.strictEqual(g1.groupDiv.classList.contains("is-collapsed"), true);
  assert.strictEqual(g2.groupDiv.classList.contains("is-collapsed"), true);
  assert.strictEqual(foldCtrl.element.title, "Mind kinyitása");
  assert.ok(foldCtrl.element.innerHTML.includes("tabler-icon-fold-down"));

  // Expand one group manually -> button switches back to fold-up + "Mind becsukása"
  g1.groupDiv.querySelector(".fav-day-header-btn").onclick();
  assert.strictEqual(g1.groupDiv.classList.contains("is-collapsed"), false);
  summary = getGroupCollapseSummary(container, collapsedKeys);
  foldCtrl.updateState(summary.allCollapsed, summary.hasGroups);
  assert.strictEqual(foldCtrl.element.title, "Mind becsukása");
  assert.ok(foldCtrl.element.innerHTML.includes("tabler-icon-fold-up"));
}

setupTestEnv();
testNormalizeAndParseSortLevels();
testMultiLevelSorting();
testAll40SortPermutations();
testDynamicGrouping();
testFilterAiringsByQuery();
testMultiSortControlsCollisionHandling();
await testLiveSearchBoxBehavior();
testCollapsibleGroupsAndFoldButton();
console.log("All airing-list tests passed!");
