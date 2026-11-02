import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isSimpleRule, outputKeys, validateSteps } from '../apps/main/src/windows/panel/outputSequence.ts';
import { buildWires } from '../apps/main/src/windows/panel/hkbWires.ts';

const q = { kind: 'keyboard' as const, code: 81 };
const e = { kind: 'keyboard' as const, code: 69 };
const r = { kind: 'keyboard' as const, code: 82 };

test('相同首步不能让多步队列进入横版快捷编辑', () => {
  assert.equal(isSimpleRule({ trigger_key: q, steps: [{ key: q, interval_ms: 50 }] }), true);
  assert.equal(isSimpleRule({ trigger_key: q, steps: [{ key: q, interval_ms: 50 }, { key: e, interval_ms: 100 }] }), false);
});

test('重复输出保留步骤顺序，走线按全部输出键去重', () => {
  const steps = [{ key: q, interval_ms: 50 }, { key: q, interval_ms: 50 }, { key: e, interval_ms: 100 }];
  assert.deepEqual(outputKeys({ steps }), [q, e]);
  const wires = buildWires([{ id: 'queue', enabled: true, mode: 'toggle', trigger_key: r, stop_key: null, steps }]);
  assert.deepEqual(wires.map((wire) => wire.to), ['keyboard:81', 'keyboard:69']);
});

test('后续步骤、空列表和容量都必须通过校验', () => {
  assert.ok(validateSteps([], 10, 10000));
  assert.ok(validateSteps([{ key: q, interval_ms: 50 }, { key: e, interval_ms: 0 }], 10, 10000));
  assert.ok(validateSteps(Array.from({ length: 257 }, () => ({ key: q, interval_ms: 50 })), 10, 10000));
  assert.equal(validateSteps([{ key: q, interval_ms: 50 }, { key: e, interval_ms: 1500 }], 10, 10000), undefined);
});
