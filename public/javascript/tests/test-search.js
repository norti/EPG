import assert from 'node:assert';
import { SearchUtils } from '../utils/search.js';

function testSzuperzsaruRegression() {
  // Must match actual Szuperzsaru variants and typos
  assert.strictEqual(SearchUtils.fuzzyMatch('Szuperzsaru', 'Szuperzsaru'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Szuperzsaru 2.', 'szuperzsaru'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Szuper zsaru', 'Szuperzsaru'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Szuperzsaru', 'Szuper zsaru'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Szuperzsaru', 'Szuprezsaru'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Szuperzsaru', 'Szuperzsaaru'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Szuperzsaru', 'Szuperzsar'), true);

  // Must NOT match unrelated titles containing substrings of "szuperzsaru" (e.g. "a", "per", "u", "szuper", "zsaru")
  const falsePositives = [
    'A mi kis falunk',
    '21 Jump Street - A kopasz osztag',
    '24 óra a halálig',
    '"Amiről a kövek mesélnek"',
    'A szupercsapat',
    'A belleville-i zsaru',
    'Az utolsó zsaru',
    'Pay-Per-View',
    'Smile of a child (English)',
    'a.tv',
    '2026 FIFA U-17 World Cup',
  ];
  for (const title of falsePositives) {
    assert.strictEqual(
      SearchUtils.fuzzyMatch(title, 'Szuperzsaru'),
      false,
      `Expected "${title}" NOT to match "Szuperzsaru"`,
    );
  }
}

function testDiacriticsAndSpecialChars() {
  assert.strictEqual(SearchUtils.fuzzyMatch('Híradó', 'hirado'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('ATV Híradó Extra', 'hirado'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Dűne', 'dune'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Weiß', 'weiss'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Łódź', 'lodz'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Đakovo', 'dakovo'), true);
}

function testShortTokenAndFuzzyThresholds() {
  // 4-char query "Dűne" must not fuzzy-match "Duna" or "Lune"
  assert.strictEqual(SearchUtils.fuzzyMatch('Duna TV', 'Dűne'), false);
  assert.strictEqual(SearchUtils.fuzzyMatch('Duna World', 'Dűne'), false);
  assert.strictEqual(SearchUtils.fuzzyMatch('A Duna - Árral szemben', 'Dűne'), false);
  assert.strictEqual(SearchUtils.fuzzyMatch('A természet dühe', 'Dűne'), false);
  assert.strictEqual(SearchUtils.fuzzyMatch('Des matériaux vivants sur la Lune', 'Dűne'), false);
  assert.strictEqual(SearchUtils.fuzzyMatch('Dune: Part Two', 'Dűne'), true);

  // 5-char query "Fargo" must not match "Margo" / "Argo" / "Faragó" under default minFuzzyLength=6
  assert.strictEqual(SearchUtils.fuzzyMatch('Fargo', 'Fargo'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Tales of Wells Fargo', 'Fargo'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('ABC - duckBC, Margo és Félix', 'Fargo'), false);
  assert.strictEqual(SearchUtils.fuzzyMatch('Argo', 'Fargo'), false);
  assert.strictEqual(SearchUtils.fuzzyMatch('Beszélgetés Faragó Andrással', 'Fargo'), false);

  // Configurable minFuzzyLength=5 allows 5-char typo matching when explicitly requested
  assert.strictEqual(
    SearchUtils.fuzzyMatch('Fargo', 'Fagro', { minFuzzyLength: 5 }),
    true,
  );

  // 6+ char typo tolerance
  assert.strictEqual(SearchUtils.fuzzyMatch('Mátrix', 'Matirx'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Keresztapa', 'Keresztpaa'), true);

  // fuzzy: false disables edit-distance matching
  assert.strictEqual(
    SearchUtils.fuzzyMatch('Szuperzsaru', 'Szuprezsaru', { fuzzy: false }),
    false,
  );
}

function testTokenizationAndPunctuation() {
  assert.strictEqual(
    SearchUtils.fuzzyMatch('Spider Man: Hazatérés', 'Spider-Man Hazatérés'),
    true,
  );
  assert.strictEqual(
    SearchUtils.fuzzyMatch('Spider-Man: No Way Home', 'Spider Man'),
    true,
  );
  assert.strictEqual(
    SearchUtils.fuzzyMatch('Spider-Man: No Way Home', 'Spiderman'),
    true,
  );
  assert.strictEqual(
    SearchUtils.fuzzyMatch('X-Men: Az elsők', 'xmen'),
    true,
  );
  assert.strictEqual(
    SearchUtils.fuzzyMatch('"Amiről a kövek mesélnek" - Történelmi városok', 'Amiről a kövek'),
    true,
  );
  assert.strictEqual(
    SearchUtils.fuzzyMatch('28 évvel később – A csonttemplom', 'csonttemplom'),
    true,
  );
  assert.strictEqual(
    SearchUtils.fuzzyMatch('Vigyázat, csalok!', 'csalok'),
    true,
  );

  // Compact join must not match across unrelated short tokens like "V./2."
  assert.strictEqual(
    SearchUtils.fuzzyMatch('A Mancs őrjárat V./2.', 'TV2'),
    false,
  );

  // Punctuation-only queries must not match everything
  assert.strictEqual(SearchUtils.fuzzyMatch('Híradó', '---'), false);
  assert.strictEqual(SearchUtils.fuzzyMatch('Híradó', '???'), false);
  assert.strictEqual(SearchUtils.fuzzyMatch('Mozi+', '+'), true);
}

function testShortTokensInMultiWordQueries() {
  assert.strictEqual(SearchUtils.fuzzyMatch('Piedone, a zsaru', 'a zsaru'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('A belleville-i zsaru', 'a zsaru'), true);
  // 1-char token "a" must not match inside "csapat" or "baratok"
  assert.strictEqual(SearchUtils.fuzzyMatch('Csapat zsaru', 'a zsaru'), false);
  // 2-char token "az" uses prefix match, not arbitrary mid-word match
  assert.strictEqual(SearchUtils.fuzzyMatch('Az utolsó zsaru', 'az zsaru'), true);
  assert.strictEqual(SearchUtils.fuzzyMatch('Ház és zsaru', 'az zsaru'), false);
}

function testPreparedQueryAndEdgeCases() {
  const emptyPrep = SearchUtils.prepareQuery('   ');
  assert.strictEqual(emptyPrep.isEmpty, true);
  assert.strictEqual(SearchUtils.matchPrepared('Bármi', emptyPrep), true);
  assert.strictEqual(SearchUtils.matchPrepared('', emptyPrep), true);

  const prep = SearchUtils.prepareQuery('Szuperzsaru');
  assert.strictEqual(SearchUtils.matchPrepared('', prep), false);
  assert.strictEqual(SearchUtils.matchPrepared(null, prep), false);
  assert.strictEqual(SearchUtils.matchPrepared('Szuperzsaru', prep), true);
  assert.strictEqual(SearchUtils.matchPrepared('A mi kis falunk', prep), false);

  assert.strictEqual(SearchUtils.damerauLevenshtein('szuprezsaru', 'szuperzsaru', 2), 1);
  assert.strictEqual(SearchUtils.damerauLevenshtein('abcdef', 'a', 2), 3);
}

testSzuperzsaruRegression();
testDiacriticsAndSpecialChars();
testShortTokenAndFuzzyThresholds();
testTokenizationAndPunctuation();
testShortTokensInMultiWordQueries();
testPreparedQueryAndEdgeCases();
console.log('All search tests passed!');
