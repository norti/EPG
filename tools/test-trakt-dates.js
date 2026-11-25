const assert = require('assert');
const TraktDates = require('../public/javascript/trakt-dates.js');

function testAllWeekdays() {
  // 2026-11-09 is Monday .. 2026-11-15 is Sunday
  const expected = [
    { date: '2026-11-09T12:00:00', today: '2026-11-09', tomorrow: '2026-11-10', fStart: '2026-11-11', fEnd: '2026-11-15' }, // Mon
    { date: '2026-11-10T12:00:00', today: '2026-11-10', tomorrow: '2026-11-11', fStart: '2026-11-12', fEnd: '2026-11-15' }, // Tue
    { date: '2026-11-11T12:00:00', today: '2026-11-11', tomorrow: '2026-11-12', fStart: '2026-11-13', fEnd: '2026-11-15' }, // Wed
    { date: '2026-11-12T12:00:00', today: '2026-11-12', tomorrow: '2026-11-13', fStart: '2026-11-14', fEnd: '2026-11-15' }, // Thu
    { date: '2026-11-13T12:00:00', today: '2026-11-13', tomorrow: '2026-11-14', fStart: '2026-11-15', fEnd: '2026-11-15' }, // Fri (single day)
    { date: '2026-11-14T12:00:00', today: '2026-11-14', tomorrow: '2026-11-15', fStart: '2026-11-16', fEnd: '2026-11-22' }, // Sat (next Mon..Sun)
    { date: '2026-11-15T12:00:00', today: '2026-11-15', tomorrow: '2026-11-16', fStart: '2026-11-17', fEnd: '2026-11-22' }, // Sun (next Tue..Sun)
  ];

  for (const tc of expected) {
    const r = TraktDates.getWeekRange(new Date(tc.date));
    assert.strictEqual(r.todayKey, tc.today, `today mismatch for ${tc.date}`);
    assert.strictEqual(r.tomorrowKey, tc.tomorrow, `tomorrow mismatch for ${tc.date}`);
    assert.strictEqual(r.furtherStartKey, tc.fStart, `furtherStart mismatch for ${tc.date}`);
    assert.strictEqual(r.furtherEndKey, tc.fEnd, `furtherEnd mismatch for ${tc.date}`);
  }
}

function testMonthAndYearBoundaries() {
  // Month boundary: 2026-11-30 (Monday) -> tomorrow 2026-12-01, further 2026-12-02..2026-12-06
  const nov30 = TraktDates.getWeekRange(new Date('2026-11-30T23:30:00'));
  assert.strictEqual(nov30.todayKey, '2026-11-30');
  assert.strictEqual(nov30.tomorrowKey, '2026-12-01');
  assert.strictEqual(nov30.furtherStartKey, '2026-12-02');
  assert.strictEqual(nov30.furtherEndKey, '2026-12-06');

  // Year boundary: 2026-12-31 (Thursday) -> tomorrow 2027-01-01, further 2027-01-02..2027-01-03
  const dec31 = TraktDates.getWeekRange(new Date('2026-12-31T10:00:00'));
  assert.strictEqual(dec31.todayKey, '2026-12-31');
  assert.strictEqual(dec31.tomorrowKey, '2027-01-01');
  assert.strictEqual(dec31.furtherStartKey, '2027-01-02');
  assert.strictEqual(dec31.furtherEndKey, '2027-01-03');

  // Leap year Feb 28 (2024-02-28 Wednesday) -> tomorrow 2024-02-29, further 2024-03-01..2024-03-03
  const feb28 = TraktDates.getWeekRange(new Date('2024-02-28T12:00:00'));
  assert.strictEqual(feb28.tomorrowKey, '2024-02-29');
  assert.strictEqual(feb28.furtherStartKey, '2024-03-01');
  assert.strictEqual(feb28.furtherEndKey, '2024-03-03');
}

function testDstTransitions() {
  // Spring DST in Europe/Budapest: 2026-03-29 (Sunday)
  const springDst = TraktDates.getWeekRange(new Date('2026-03-29T01:30:00'));
  assert.strictEqual(springDst.todayKey, '2026-03-29');
  assert.strictEqual(springDst.tomorrowKey, '2026-03-30');
  assert.strictEqual(springDst.furtherStartKey, '2026-03-31');
  assert.strictEqual(springDst.furtherEndKey, '2026-04-05');

  // Fall DST in Europe/Budapest: 2026-10-25 (Sunday)
  const fallDst = TraktDates.getWeekRange(new Date('2026-10-25T02:30:00'));
  assert.strictEqual(fallDst.todayKey, '2026-10-25');
  assert.strictEqual(fallDst.tomorrowKey, '2026-10-26');
  assert.strictEqual(fallDst.furtherStartKey, '2026-10-27');
  assert.strictEqual(fallDst.furtherEndKey, '2026-11-01');
}

function testSplitDashboardEpisodesNoDuplicates() {
  const now = new Date('2026-11-14T12:00:00'); // Saturday
  const episodes = [
    { episodeTraktId: 1, firstAired: new Date('2026-11-14T20:00:00').toISOString() }, // today
    { episodeTraktId: 2, firstAired: new Date('2026-11-15T20:00:00').toISOString() }, // tomorrow
    { episodeTraktId: 3, firstAired: new Date('2026-11-16T20:00:00').toISOString() }, // further (Mon)
    { episodeTraktId: 4, firstAired: new Date('2026-11-22T20:00:00').toISOString() }, // further (next Sun)
    { episodeTraktId: 5, firstAired: new Date('2026-11-23T20:00:00').toISOString() }, // outside range
    { episodeTraktId: 6, firstAired: new Date('2026-11-13T20:00:00').toISOString() }, // yesterday (outside)
  ];

  const split = TraktDates.splitDashboardEpisodes(episodes, now);
  assert.deepStrictEqual(split.today.map((e) => e.episodeTraktId), [1]);
  assert.deepStrictEqual(split.tomorrow.map((e) => e.episodeTraktId), [2]);
  assert.deepStrictEqual(split.further.map((e) => e.episodeTraktId), [3, 4]);
}

function testCompareEpisodesByStartTime() {
  const list = [
    { episodeTraktId: 3, showTitle: 'Beta', firstAired: new Date('2026-11-25T21:00:00').toISOString() },
    { episodeTraktId: 1, showTitle: 'Gamma', firstAired: new Date('2026-11-24T20:00:00').toISOString() },
    { episodeTraktId: 2, showTitle: 'Alpha', firstAired: new Date('2026-11-25T09:00:00').toISOString() },
    { episodeTraktId: 0, showTitle: 'Old', firstAired: new Date('2026-11-10T03:00:00').toISOString() },
  ];
  list.sort(TraktDates.compareEpisodesByStartTime);
  assert.deepStrictEqual(list.map((e) => e.episodeTraktId), [0, 1, 2, 3]);
}

function testFormatDashboardReleaseMeta() {
  const now = new Date('2026-11-25T15:00:00');
  const labels = { yesterday: 'Tegnap' };

  assert.strictEqual(
    TraktDates.formatDashboardReleaseMeta(new Date('2026-11-25T09:30:00').toISOString(), now, labels),
    '09:30'
  );
  assert.strictEqual(
    TraktDates.formatDashboardReleaseMeta(new Date('2026-11-25T21:00:00').toISOString(), now, labels),
    '21:00'
  );
  assert.strictEqual(
    TraktDates.formatDashboardReleaseMeta(new Date('2026-11-24T20:00:00').toISOString(), now, labels),
    'Tegnap'
  );
  assert.strictEqual(
    TraktDates.formatDashboardReleaseMeta(new Date('2026-11-24T20:00:00').toISOString(), now, { yesterday: 'Yesterday' }),
    'Yesterday'
  );
  assert.strictEqual(
    TraktDates.formatDashboardReleaseMeta(new Date('2026-11-18T03:00:00').toISOString(), now, labels),
    '11.18.'
  );
  assert.strictEqual(
    TraktDates.formatDashboardReleaseMeta(new Date('2026-01-05T03:00:00').toISOString(), now, labels),
    '01.05.'
  );
  assert.strictEqual(
    TraktDates.formatDashboardReleaseMeta(new Date('2025-12-20T03:00:00').toISOString(), now, labels),
    '2025.12.20.'
  );
}

function testGetFurtherWeekRangesUpTo30Days() {
  // Wed 2026-11-25 -> further starts Fri 2026-11-27, horizon 2026-12-25
  const now = new Date('2026-11-25T12:00:00');
  const weeks = TraktDates.getFurtherWeekRanges(now, 30);
  assert.deepStrictEqual(
    weeks.map((w) => [w.startKey, w.endKey]),
    [
      ['2026-11-27', '2026-11-29'],
      ['2026-11-30', '2026-12-06'],
      ['2026-12-07', '2026-12-13'],
      ['2026-12-14', '2026-12-20'],
      ['2026-12-21', '2026-12-25'],
    ]
  );

  // Verify contiguous coverage (no gaps, no overlaps) across all 7 weekdays
  for (let d = 9; d <= 15; d++) {
    const dt = new Date(`2026-11-${String(d).padStart(2, '0')}T12:00:00`);
    const ws = TraktDates.getFurtherWeekRanges(dt, 30);
    assert(ws.length >= 4, `expected at least 4 weeks for day ${d}`);
    assert.strictEqual(ws[0].startKey, TraktDates.toLocalYmd(TraktDates.addLocalDays(dt, 2)));
    assert.strictEqual(ws[ws.length - 1].endKey, TraktDates.toLocalYmd(TraktDates.addLocalDays(dt, 30)));
    for (let i = 1; i < ws.length; i++) {
      const expectedStart = TraktDates.toLocalYmd(TraktDates.addLocalDays(ws[i - 1].end, 1));
      assert.strictEqual(ws[i].startKey, expectedStart, `gap/overlap between week ${i - 1} and ${i}`);
    }
  }

  // Filter episodes across pages and verify sorting
  const eps = [
    { episodeTraktId: 10, firstAired: new Date('2026-12-02T22:00:00').toISOString() },
    { episodeTraktId: 11, firstAired: new Date('2026-11-30T03:00:00').toISOString() },
    { episodeTraktId: 12, firstAired: new Date('2026-12-25T20:00:00').toISOString() },
    { episodeTraktId: 13, firstAired: new Date('2026-12-26T20:00:00').toISOString() }, // beyond 30d
  ];
  assert.deepStrictEqual(
    TraktDates.filterEpisodesByDateRange(eps, weeks[1].startKey, weeks[1].endKey).map((e) => e.episodeTraktId),
    [11, 10]
  );
  assert.deepStrictEqual(
    TraktDates.filterEpisodesByDateRange(eps, weeks[4].startKey, weeks[4].endKey).map((e) => e.episodeTraktId),
    [12]
  );
}

testAllWeekdays();
testMonthAndYearBoundaries();
testDstTransitions();
testSplitDashboardEpisodesNoDuplicates();
testCompareEpisodesByStartTime();
testFormatDashboardReleaseMeta();
testGetFurtherWeekRangesUpTo30Days();
console.log('All trakt-dates tests passed!');
