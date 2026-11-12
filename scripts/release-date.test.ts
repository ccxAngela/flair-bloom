import assert from 'node:assert/strict';
import test from 'node:test';
import { formatReleaseDate } from './release-date.ts';

test('发版日期按目标本地时区而不是 UTC 日期生成', () => {
  const instant = new Date('2026-11-11T16:30:00Z');

  assert.equal(formatReleaseDate(instant, 'Asia/Shanghai'), '2026-11-12');
  assert.equal(formatReleaseDate(instant, 'UTC'), '2026-11-11');
});
