import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  dateToLocalIsoWithOffset,
  buildCalendarSummary,
  normalizeCalendarChannelId,
  buildCalendarEpgKey,
  getGoogleCalendarIconSvg,
} from "../google/google-calendar-utils.js";

describe("dateToLocalIsoWithOffset", () => {
  it("formats UTC Date with positive timezone offset (+01:00 CET)", () => {
    const d = new Date("2026-11-26T19:00:00Z");
    assert.equal(dateToLocalIsoWithOffset(d, -60), "2026-11-26T20:00:00+01:00");
  });

  it("formats UTC Date with negative timezone offset (-05:00 EST) across midnight", () => {
    const d = new Date("2026-11-26T02:30:00Z");
    assert.equal(dateToLocalIsoWithOffset(d, 300), "2026-11-25T21:30:00-05:00");
  });

  it("formats UTC Date with zero offset (+00:00)", () => {
    const d = new Date("2026-06-15T00:00:00Z");
    assert.equal(dateToLocalIsoWithOffset(d, 0), "2026-06-15T00:00:00+00:00");
  });

  it("returns empty string for invalid or non-Date inputs", () => {
    assert.equal(dateToLocalIsoWithOffset(null), "");
    assert.equal(dateToLocalIsoWithOffset(new Date("invalid")), "");
  });
});

describe("buildCalendarSummary", () => {
  it("appends Hungarian TMDB title when different from programme title", () => {
    assert.equal(
      buildCalendarSummary("Inception", { huTitle: "Eredet" }),
      "Inception (Eredet)",
    );
  });

  it("does not duplicate identical titles (case-insensitive)", () => {
    assert.equal(
      buildCalendarSummary("Eredet", { huTitle: "eredet" }),
      "Eredet",
    );
    assert.equal(buildCalendarSummary("Híradó", null), "Híradó");
  });
});

describe("buildCalendarEpgKey & normalizeCalendarChannelId", () => {
  it("normalizes __ALL_FAVORITES__ composite channel IDs identically to normal source IDs", () => {
    const start = new Date("2026-11-26T20:00:00Z");
    const normalKey = buildCalendarEpgKey("porthu", "398", start);
    const favKey = buildCalendarEpgKey("porthu", "porthu::398", start);
    assert.equal(normalKey, favKey);
    assert.equal(normalKey, "porthu|398|1795723200");
    assert.equal(normalizeCalendarChannelId("porthu", "porthu::398"), "398");
    assert.equal(normalizeCalendarChannelId("porthu", "other::398"), "other::398");
  });
});

describe("getGoogleCalendarIconSvg", () => {
  it("renders dynamic day number and unique gradient IDs per call", () => {
    const svg1 = getGoogleCalendarIconSvg(26, 16);
    const svg2 = getGoogleCalendarIconSvg(5, 12);
    assert.match(svg1, />26<\/text>/);
    assert.match(svg2, />5<\/text>/);
    assert.notEqual(svg1, svg2);
  });
});
