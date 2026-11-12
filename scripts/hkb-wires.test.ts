// 横版走线几何的纯函数断言。前端没有测试框架，用 Node 内置 test runner + 类型剥离直接跑：
// `pnpm test:ui`。
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import {
  buildWires,
  chamferPath,
  countCrossings,
  FAN_GAP,
  fanOffset,
  placeWires,
  type PlacedWire,
  type Rect,
  wiredKeys,
  type WireRule,
} from '../apps/main/src/windows/panel/hkbWires.ts';
import { keyToken } from '../apps/main/src/windows/panel/keyToken.ts';

const kb = (code: number) => ({ kind: 'keyboard', code }) as unknown as WireRule['trigger_key'];

function rule(
  id: string,
  trigger: number,
  target: number,
  mode: 'hold' | 'toggle' = 'hold',
  stop?: number,
): WireRule {
  return {
    id,
    enabled: true,
    trigger_key: kb(trigger),
    steps: [{ key: kb(target), interval_ms: 50 }],
    mode,
    stop_key: stop === undefined ? null : kb(stop),
  };
}

test('trigger == target 的单键规则不出边', () => {
  assert.deepEqual(buildWires([rule('a', 0x51, 0x51), rule('b', 0x45, 0x45, 'toggle')]), []);
});

test('trigger != target 出一条 target 边，保留模式与启用态', () => {
  const r = { ...rule('a', 0x51, 0x45, 'toggle'), enabled: false };
  assert.deepEqual(buildWires([r]), [
    {
      ruleId: 'a',
      from: keyToken(kb(0x51)),
      to: keyToken(kb(0x45)),
      kind: 'target',
      mode: 'toggle',
      enabled: false,
    },
  ]);
});

test('切换连发独立停止键出 stop 边，停止键 == 启动键不出', () => {
  const wires = buildWires([
    rule('a', 0x51, 0x51, 'toggle', 0x58),
    rule('b', 0x46, 0x46, 'toggle', 0x46),
  ]);
  assert.equal(wires.length, 1);
  assert.equal(wires[0].kind, 'stop');
  assert.equal(wires[0].to, keyToken(kb(0x58)));
});

test('长按规则的 stop_key 不出边', () => {
  assert.deepEqual(buildWires([rule('a', 0x51, 0x51, 'hold', 0x58)]), []);
});

test('参与键集合是所有边的两端', () => {
  const keys = wiredKeys(
    buildWires([rule('a', 0x51, 0x45), rule('b', 0x46, 0x46, 'toggle', 0x58)]),
  );
  assert.deepEqual([...keys].sort(), [kb(0x51), kb(0x45), kb(0x46), kb(0x58)].map(keyToken).sort());
});

test('并行扇出偏移居中对称、间距为 FAN_GAP', () => {
  assert.equal(fanOffset(0, 1), 0);
  const three = [0, 1, 2].map((i) => fanOffset(i, 3));
  assert.deepEqual(three, [-FAN_GAP, 0, FAN_GAP]);
  const four = [0, 1, 2, 3].map((i) => fanOffset(i, 4));
  assert.equal(four[0], -four[3]);
  assert.equal(four[1], -four[2]);
  assert.equal(four[1] - four[0], FAN_GAP);
});

test('切角路径：两端不动，每个拐角多出一段 45° 斜边', () => {
  const d = chamferPath([
    [0, 0],
    [0, 30],
    [50, 30],
    [50, 60],
  ]);
  assert.equal(d, 'M0 0 L0 24 L6 30 L44 30 L50 36 L50 60');
});

test('空折线不出路径', () => {
  assert.equal(chamferPath([]), '');
});

test('悬停键：出去的与指向它的分开；端点不在图上的不出路径', () => {
  const wires = buildWires([
    rule('out1', 0x51, 0x45),
    rule('out2', 0x51, 0x52),
    rule('in1', 0x46, 0x51),
    rule('offmap', 0x51, 0x5a),
  ]);
  const rects = new Map([
    [keyToken(kb(0x51)), { left: 0, top: 0, width: 40, height: 40 }],
    [keyToken(kb(0x45)), { left: 200, top: 100, width: 40, height: 40 }],
    [keyToken(kb(0x52)), { left: 100, top: 100, width: 40, height: 40 }],
    [keyToken(kb(0x46)), { left: 300, top: 100, width: 40, height: 40 }],
  ]);
  const placed = placeWires(wires, keyToken(kb(0x51)), (t) => rects.get(t));
  assert.deepEqual(
    placed.map((p) => `${p.dir}:${p.wire.ruleId}`),
    ['out:out2', 'out:out1', 'in:in1'],
  );
  assert.equal(crossings(placed), 0);
});

// 真实键盘几何：键宽 40、键距与行距 5。col / row 是格子坐标。
const cap = (col: number, row: number, w = 1): Rect => ({
  left: col * 45,
  top: row * 45,
  width: w * 40 + (w - 1) * 5,
  height: 40,
});

/** 以键名搭一张小键盘，悬停 hover，按 [from, to] 列出走线。 */
function scene(keys: Record<string, Rect>, hover: string, edges: [string, string][]) {
  const wires = edges.map(([from, to], i) => ({
    ruleId: `r${i}`,
    from,
    to,
    kind: 'target' as const,
    mode: 'hold' as const,
    enabled: true,
  }));
  const placed = placeWires(wires, hover, (t) => keys[t]);
  const by = (from: string, to: string) => {
    const i = edges.findIndex(([f, t]) => f === from && t === to);
    return placed.find((p) => p.wire.ruleId === `r${i}`)!;
  };
  return { placed, by };
}

function crossings(placed: PlacedWire[]): number {
  let n = 0;
  for (let i = 0; i < placed.length; i++) {
    for (let j = i + 1; j < placed.length; j++)
      n += countCrossings(placed[i].points, placed[j].points);
  }
  return n;
}

const numberRow = Object.fromEntries(
  ['1', '2', '3', '4', '5', '6', '7', '8', '9'].map((k, i) => [k, cap(i, 1)]),
);

test('截图场景：6 出到 7、8，4、5 进到 6——出线走上、入线走下，相邻键也走通道，零交叉', () => {
  const { placed, by } = scene(numberRow, '6', [
    ['6', '7'],
    ['6', '8'],
    ['4', '6'],
    ['5', '6'],
  ]);
  assert.equal(crossings(placed), 0);
  const rowTop = 45;
  const rowBottom = 85;
  for (const [f, t] of [
    ['6', '7'],
    ['6', '8'],
  ]) {
    assert.equal(by(f, t).points.length, 4);
    assert.ok(by(f, t).points[1][1] < rowTop, `${f}→${t} 走上方`);
  }
  for (const [f, t] of [
    ['4', '6'],
    ['5', '6'],
  ]) {
    assert.ok(by(f, t).points[1][1] > rowBottom, `${f}→${t} 走下方`);
  }
  // 相邻的 7 在最内层，8 套在外面
  assert.ok(by('6', '8').points[1][1] < by('6', '7').points[1][1]);
  // 入线反向：起点在启动键 4 的下沿
  assert.deepEqual(by('4', '6').start[1], rowBottom);
  assert.ok(by('4', '6').start[0] < cap(3, 1).left + 40);
});

test('同侧多条同行走线按跨度嵌套：远的通道更外、出线点更靠中心，不交叉', () => {
  const { placed, by } = scene(numberRow, '3', [
    ['3', '5'],
    ['3', '7'],
    ['3', '9'],
  ]);
  assert.equal(crossings(placed), 0);
  const [near, mid, far] = [by('3', '5'), by('3', '7'), by('3', '9')];
  assert.ok(far.points[1][1] < mid.points[1][1] && mid.points[1][1] < near.points[1][1]);
  assert.ok(far.start[0] < mid.start[0] && mid.start[0] < near.start[0]);
});

test('左右两向的同行出线分在悬停键上沿两半，互不交叉', () => {
  const { placed, by } = scene(numberRow, '5', [
    ['5', '1'],
    ['5', '9'],
  ]);
  assert.equal(crossings(placed), 0);
  const c = cap(4, 1).left + 20;
  assert.ok(by('5', '1').start[0] < c && by('5', '9').start[0] > c);
});

test('跨行多条同向：跨度远的在内层，末段不切过别的横段', () => {
  const keys = { ...numberRow, A: cap(4, 3), B: cap(6, 3), C: cap(8, 3) };
  const { placed } = scene(keys, '2', [
    ['2', 'A'],
    ['2', 'B'],
    ['2', 'C'],
  ]);
  assert.equal(crossings(placed), 0);
  for (const p of placed) assert.ok(p.points[0][1] === 85, '从下沿出');
});

test('走线层级低于键帽，跨行路径不会压在中间键帽表面', () => {
  const css = readFileSync(
    new URL('../apps/main/src/windows/panel/HorizontalLayout.css', import.meta.url),
    'utf8',
  );
  const z = (selector: string) => {
    const escaped = selector.replace('.', '\\.');
    const body = css.match(new RegExp(`${escaped}\\s*\\{([\\s\\S]*?)\\}`))?.[1];
    const value = body?.match(/z-index:\s*(-?\d+)/)?.[1];
    return value === undefined ? undefined : Number(value);
  };

  const capZ = z('.hkb-cap');
  const wireZ = z('.hkb-wires');
  assert.notEqual(capZ, undefined);
  assert.notEqual(wireZ, undefined);
  assert.ok(capZ! > wireZ!, `键帽层级 ${capZ} 应高于走线层级 ${wireZ}`);
});

test('默认侧会交叉时改走另一侧', () => {
  // 6 → 9 同行默认走上方；6 → X 去上两行、横坐标落在 6 与 9 之间，末段会切过上方通道
  const keys = { ...numberRow, X: cap(6, -1) };
  const { placed, by } = scene(keys, '6', [
    ['6', '9'],
    ['6', 'X'],
  ]);
  assert.equal(crossings(placed), 0);
  assert.ok(by('6', '9').points[1][1] > 85, '同行那条让到下方');
});

test('同一对端的多条走线在两头都错开，不重叠', () => {
  const { placed } = scene(numberRow, '2', [
    ['2', '6'],
    ['2', '6'],
  ]);
  assert.equal(crossings(placed), 0);
  assert.notDeepEqual(placed[0].end, placed[1].end);
});

test('同侧同向走线多到排不下时分到两侧，同侧出线点间距不小于平行判重阈值', () => {
  const { placed } = scene(numberRow, '1', [
    ['1', '2'],
    ['1', '3'],
    ['1', '4'],
    ['1', '5'],
  ]);
  assert.equal(crossings(placed), 0);
  const rowTop = 45;
  for (const sideOf of [(y: number) => y < rowTop, (y: number) => y >= rowTop]) {
    const xs = placed
      .filter((p) => sideOf(p.points[1][1]))
      .map((p) => p.start[0])
      .sort((a, b) => a - b);
    for (let i = 1; i < xs.length; i++) assert.ok(xs[i] - xs[i - 1] >= 3.5, `出线点 ${xs} 太挤`);
  }
});

test('平行段贴得太近算重叠，拉开到阈值以上不算', () => {
  const h = (y: number): [number, number][] => [
    [0, y],
    [10, y],
  ];
  assert.equal(countCrossings(h(0), h(2)), 1);
  assert.equal(countCrossings(h(0), h(4)), 0);
});

test('折线交叉计数：十字相交算一次，端点相接不算，同线重叠算一次', () => {
  assert.equal(
    countCrossings(
      [
        [0, 5],
        [10, 5],
      ],
      [
        [5, 0],
        [5, 10],
      ],
    ),
    1,
  );
  assert.equal(
    countCrossings(
      [
        [0, 0],
        [10, 0],
      ],
      [
        [10, 0],
        [10, 10],
      ],
    ),
    0,
  );
  assert.equal(
    countCrossings(
      [
        [0, 0],
        [10, 0],
      ],
      [
        [5, 0],
        [15, 0],
      ],
    ),
    1,
  );
});
