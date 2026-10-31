import assert from 'node:assert/strict';
import test from 'node:test';
import { collectLiveEnvironment } from '../apps/main/src/windows/panel/environment.ts';

test('环境采集部分失败仍保留其余证据，并附带失败来源', async () => {
  const original = globalThis.window;
  globalThis.window = {
    innerWidth: 1072, innerHeight: 603, devicePixelRatio: 1.5,
    visualViewport: { scale: 1 },
    __TAURI_INTERNALS__: {
      invoke: async (command: string) => {
        if (command === 'get_app_status') return { input_mode: 'sendinput' };
        throw new Error('display unavailable');
      },
    },
  } as unknown as Window;
  try {
    const result = await collectLiveEnvironment();
    assert.equal(result.status?.input_mode, 'sendinput');
    assert.equal(result.display, null);
    assert.deepEqual(result.errors, ['display: Error: display unavailable']);
    assert.deepEqual(result.frontend.viewport_css, { width: 1072, height: 603 });
    assert.equal(result.frontend.device_pixel_ratio, 1.5);
  } finally { globalThis.window = original; }
});

test('再次采集重新读取缩放，不复用旧快照；状态失败也不丢显示信息', async () => {
  const original = globalThis.window;
  let textScale = 100;
  globalThis.window = {
    innerWidth: 405, innerHeight: 720, devicePixelRatio: 1,
    __TAURI_INTERNALS__: {
      invoke: async (command: string) => {
        if (command === 'get_app_status') throw new Error('status unavailable');
        return { text_scale_percent: textScale };
      },
    },
  } as unknown as Window;
  try {
    assert.equal((await collectLiveEnvironment()).display?.text_scale_percent, 100);
    textScale = 150;
    const result = await collectLiveEnvironment();
    assert.equal(result.display?.text_scale_percent, 150);
    assert.equal(result.status, null);
    assert.deepEqual(result.errors, ['status: Error: status unavailable']);
    assert.equal(result.frontend.visual_viewport_scale, null);
  } finally { globalThis.window = original; }
});
