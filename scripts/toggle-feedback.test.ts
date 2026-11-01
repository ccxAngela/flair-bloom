import assert from 'node:assert/strict';
import test from 'node:test';
import { createToggleFeedbackHandler } from '../apps/main/src/windows/panel/toggleFeedback.ts';

const key = { kind: 'keyboard', code: 70 } as const;

test('全局停止后，迟到的开始通知不能覆盖停止播报', () => {
  let enabled = true;
  const played: boolean[] = [];
  const receive = createToggleFeedbackHandler({
    isReady: () => true,
    isEnabled: () => enabled,
    play: (_, started) => played.push(started),
  });
  receive({ key, started: true });
  enabled = false;
  receive({ key, started: true });
  receive({ key, started: false });
  assert.deepEqual(played, [true]);
  enabled = true;
  receive({ key, started: true });
  receive({ key, started: false });
  assert.deepEqual(played, [true, true, false]);
});

test('启动和切配置期间跳过启停语音，恢复后读取最新状态', () => {
  let ready = false;
  let played = 0;
  const receive = createToggleFeedbackHandler({
    isReady: () => ready,
    isEnabled: () => true,
    play: () => played++,
  });
  receive({ key, started: true });
  assert.equal(played, 0);
  ready = true;
  receive({ key, started: true });
  assert.equal(played, 1);
  ready = false;
  receive({ key, started: false });
  assert.equal(played, 1);
});
