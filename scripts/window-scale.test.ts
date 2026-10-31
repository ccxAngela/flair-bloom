// 面板窗口等比缩放的纯计算：内容比例、工作区上限与存储净化。拖边锁比例在 Rust 侧 commands/window.rs。
import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  correctedWindowSize,
  fitScale,
  LAYOUT_BASE,
  maximizedRect,
  maxScale,
  reachedMaximum,
  restoreUnderCursor,
  sanitizeScales,
  scaledSize,
} from '../apps/main/src/windows/panel/windowScale.ts';

const V = LAYOUT_BASE.vertical;
const INF = Number.POSITIVE_INFINITY;

test('文字大小热切换使原生客户区少 4px 时恢复完整画布', () => {
  assert.deepEqual(correctedWindowSize({ width: 1072, height: 599 }, LAYOUT_BASE.horizontal, INF), {
    width: 1072,
    height: 603,
  });
  assert.deepEqual(correctedWindowSize({ width: 405, height: 716 }, V, INF), V);
});

test('正常拖边的舍入误差不触发尺寸回写，最小化也不复位', () => {
  assert.equal(correctedWindowSize({ width: 1072, height: 602.7 }, LAYOUT_BASE.horizontal, INF), undefined);
  assert.equal(correctedWindowSize({ width: 810, height: 1440 }, V, INF), undefined);
  assert.equal(correctedWindowSize({ width: 0, height: 0 }, V, INF), undefined);
});

test('fitScale 取宽高比例里较小的，内容不溢出', () => {
  assert.equal(fitScale({ width: 810, height: 1000 }, V, INF), 1.39);
  assert.equal(fitScale({ width: 500, height: 1440 }, V, INF), 1.23);
});

test('fitScale 不低于 1、不超过上限', () => {
  assert.equal(fitScale({ width: 300, height: 500 }, V, INF), 1);
  assert.equal(fitScale({ width: 1215, height: 2160 }, V, 2), 2);
});

test('maxScale 按工作区装得下的最大等比尺寸，向下取两位，至少 1', () => {
  // 1920×1040 工作区装竖版：高度先到顶，1040 / 720 = 1.444…
  assert.equal(maxScale({ width: 1920, height: 1040 }, V), 1.44);
  assert.equal(maxScale({ width: 1280, height: 600 }, V), 1);
});

test('scaledSize 取整', () => {
  assert.deepEqual(scaledSize(V, 1.25), { width: 506, height: 900 });
});

test('maximizedRect 取工作区装得下的最大等比尺寸并居中', () => {
  // 1920×1040 工作区（任务栏在下）装竖版：比例 1.44，高 1037 被顶住，左右留白一样多
  const r = maximizedRect({ x: 0, y: 0, width: 1920, height: 1040 }, V);
  assert.deepEqual(r, { x: 669, y: 2, width: 583, height: 1037 });
  // 副屏工作区从负坐标起
  const s = maximizedRect({ x: -1920, y: 0, width: 1920, height: 1040 }, V);
  assert.equal(s.x, 669 - 1920);
});

test('restoreUnderCursor 让鼠标停在窗口里同一相对位置', () => {
  const maximized = { x: 0, y: 0, width: 810, height: 1440 };
  // 鼠标在标题栏横向 3/4、纵向 20 px 处
  const r = restoreUnderCursor({ x: 607.5, y: 20 }, maximized, V);
  assert.deepEqual(r, { x: 304, y: 10, width: 405, height: 720 });
});

test('sanitizeScales 把缺失、非数、小于 1 的值当 1', () => {
  assert.deepEqual(sanitizeScales(undefined), { vertical: 1, horizontal: 1 });
  assert.deepEqual(sanitizeScales({ vertical: 1.5, horizontal: 0.5 }), { vertical: 1.5, horizontal: 1 });
  assert.deepEqual(sanitizeScales({ vertical: 'x', horizontal: Number.NaN }), { vertical: 1, horizontal: 1 });
});

test('reachedMaximum 拉到最大尺寸才算，工作区装不下更大的窗口时永远不算', () => {
  const target = { width: 607, height: 1080 };
  assert.equal(reachedMaximum({ width: 608, height: 1081 }, target, V), true);
  assert.equal(reachedMaximum({ width: 500, height: 889 }, target, V), false);
  // 小屏：最大化尺寸就是设计尺寸，拖到最小也等于「到顶」
  assert.equal(reachedMaximum(V, V, V), false);
});
