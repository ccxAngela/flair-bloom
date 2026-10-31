import assert from 'node:assert/strict';
import test from 'node:test';
import { completeImportWelcome, prepareImportWelcome } from '../apps/main/src/windows/panel/importWelcomeState.ts';

function memoryStore(initial: Record<string, unknown> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    async get<T>(key: string) { return data.get(key) as T | undefined; },
    async set(key: string, value: unknown) { data.set(key, value); },
    async save() {},
  };
}

test('新装用户先同意协议又退出时，下次启动仍可选择导入；选择后不再弹出', async () => {
  const store = memoryStore();
  assert.equal(await prepareImportWelcome(store), true);
  await store.set('agreed', true);
  assert.equal(await prepareImportWelcome(store), true);
  await completeImportWelcome(store);
  assert.equal(await prepareImportWelcome(store), false);
});

test('已经同意过协议的老用户不会因升级或教程版本变化收到首装邀请', async () => {
  const store = memoryStore({ agreed: true, agreement_version: 'old', tours: { introVersion: 'old' } });
  assert.equal(await prepareImportWelcome(store), false);
  assert.equal(await store.get('import_welcome_pending'), undefined);
});
