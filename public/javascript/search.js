/**
 * Reusable text normalization & fuzzy search utilities for EPG Viewer.
 * Works in both browser (window.SearchUtils) and Node.js (CommonJS require).
 */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) {
    module.exports = factory();
  } else {
    root.SearchUtils = factory();
  }
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  const NON_ALNUM_RE = /[^\p{L}\p{N}]+/u;

  const DEFAULT_OPTIONS = Object.freeze({
    fuzzy: true,
    minFuzzyLength: 6,
    twoEditMinLength: 9,
    minCompactJoinLength: 6,
  });

  function normalizeText(str) {
    if (!str) return '';
    return String(str)
      .replace(/[ßẞ]/g, 'ss')
      .replace(/[øØ]/g, 'o')
      .replace(/[łŁ]/g, 'l')
      .replace(/[đĐ]/g, 'd')
      .normalize('NFD')
      .replace(/[\u0300-\u036f]/g, '')
      .toLowerCase()
      .trim();
  }

  function tokenize(normStr) {
    if (!normStr) return [];
    return normStr.split(NON_ALNUM_RE).filter(Boolean);
  }

  // Optimal String Alignment (restricted Damerau-Levenshtein) with maxDist early exit.
  function damerauLevenshtein(a, b, maxDist = Infinity) {
    if (a === b) return 0;
    const la = a.length;
    const lb = b.length;
    if (la === 0) return lb;
    if (lb === 0) return la;
    if (Number.isFinite(maxDist) && Math.abs(la - lb) > maxDist) {
      return maxDist + 1;
    }

    let prevPrev = new Array(lb + 1);
    let prev = new Array(lb + 1);
    let curr = new Array(lb + 1);

    for (let j = 0; j <= lb; j++) {
      prev[j] = j;
    }

    for (let i = 1; i <= la; i++) {
      curr[0] = i;
      let rowMin = curr[0];
      const aChar = a.charCodeAt(i - 1);
      const aPrevChar = i > 1 ? a.charCodeAt(i - 2) : 0;

      for (let j = 1; j <= lb; j++) {
        const bChar = b.charCodeAt(j - 1);
        const cost = aChar === bChar ? 0 : 1;
        let val = Math.min(
          prev[j] + 1,
          curr[j - 1] + 1,
          prev[j - 1] + cost,
        );
        if (
          i > 1 &&
          j > 1 &&
          aChar === b.charCodeAt(j - 2) &&
          aPrevChar === bChar
        ) {
          val = Math.min(val, prevPrev[j - 2] + 1);
        }
        curr[j] = val;
        if (val < rowMin) rowMin = val;
      }

      if (Number.isFinite(maxDist) && rowMin > maxDist) {
        return maxDist + 1;
      }

      const tmp = prevPrev;
      prevPrev = prev;
      prev = curr;
      curr = tmp;
    }

    return prev[lb];
  }

  function resolveOptions(options) {
    if (!options) return DEFAULT_OPTIONS;
    return {
      fuzzy: options.fuzzy !== undefined ? Boolean(options.fuzzy) : DEFAULT_OPTIONS.fuzzy,
      minFuzzyLength:
        typeof options.minFuzzyLength === 'number'
          ? options.minFuzzyLength
          : DEFAULT_OPTIONS.minFuzzyLength,
      twoEditMinLength:
        typeof options.twoEditMinLength === 'number'
          ? options.twoEditMinLength
          : DEFAULT_OPTIONS.twoEditMinLength,
      minCompactJoinLength:
        typeof options.minCompactJoinLength === 'number'
          ? options.minCompactJoinLength
          : DEFAULT_OPTIONS.minCompactJoinLength,
    };
  }

  function prepareQuery(query, options) {
    const opts = resolveOptions(options);
    const nQuery = normalizeText(query);
    if (!nQuery) {
      return {
        isEmpty: true,
        nQuery: '',
        tokens: [],
        hasTokens: false,
        options: opts,
      };
    }
    const tokens = tokenize(nQuery);
    return {
      isEmpty: false,
      nQuery,
      tokens,
      hasTokens: tokens.length > 0,
      options: opts,
    };
  }

  function tokenMatches(tTok, qTok, opts) {
    const qLen = qTok.length;
    if (qLen === 1) {
      return tTok === qTok;
    }
    if (qLen === 2) {
      return tTok.startsWith(qTok);
    }
    if (tTok.includes(qTok)) {
      return true;
    }
    if (opts.fuzzy && qLen >= opts.minFuzzyLength && tTok.length >= opts.minFuzzyLength - 1) {
      const maxAllowed = qLen >= opts.twoEditMinLength ? 2 : 1;
      if (Math.abs(qLen - tTok.length) <= maxAllowed) {
        return damerauLevenshtein(qTok, tTok, maxAllowed) <= maxAllowed;
      }
    }
    return false;
  }

  function matchPrepared(target, prepared) {
    if (!prepared || prepared.isEmpty) return true;
    if (!target) return false;

    const nTarget = normalizeText(target);
    if (!nTarget) return false;

    // 1. Direct normalized substring match (handles case and accents, e.g. "hirado" -> "Híradó")
    if (nTarget.includes(prepared.nQuery)) return true;

    // Punctuation-only queries (e.g. "---") must not match everything
    if (!prepared.hasTokens) return false;

    const tTokens = tokenize(nTarget);
    if (tTokens.length === 0) return false;

    const qTokens = prepared.tokens;
    const opts = prepared.options;

    // 2. Adjacent-token join for compound/hyphenated single-word queries (e.g. "Spiderman" -> "Spider-Man", "xmen" -> "X-Men")
    if (qTokens.length === 1 && qTokens[0].length >= 4) {
      const qTok = qTokens[0];
      const allowSubstr = qTok.length >= opts.minCompactJoinLength;
      for (let i = 0; i < tTokens.length - 1; i++) {
        const joined = tTokens[i] + tTokens[i + 1];
        if (allowSubstr ? joined.includes(qTok) : joined === qTok) {
          return true;
        }
      }
    }

    // 3. Token-level matching (every query token must match at least one target token)
    return qTokens.every((qTok) =>
      tTokens.some((tTok) => tokenMatches(tTok, qTok, opts)),
    );
  }

  function fuzzyMatch(target, query, options) {
    const prepared = prepareQuery(query, options);
    return matchPrepared(target, prepared);
  }

  return {
    normalizeText,
    tokenize,
    damerauLevenshtein,
    prepareQuery,
    matchPrepared,
    fuzzyMatch,
  };
});
