import { outputKeys, type OutputStep } from './outputSequence.ts';
import type { KeyId } from './components/KeyCapture';
// 带扩展名：scripts/hkb-wires.test.ts 用 Node 类型剥离直接跑本文件，无扩展名的相对导入会报
// ERR_MODULE_NOT_FOUND；Vite 与 tsc（allowImportingTsExtensions）都认这种写法。
import { keyToken } from './keyToken.ts';

type BurstMode = 'hold' | 'toggle';

/** 走线只关心规则的这几个字段。 */
export interface WireRule {
  id: string;
  enabled: boolean;
  trigger_key: KeyId;
  steps: OutputStep[];
  mode: BurstMode;
  stop_key: KeyId | null;
}

/** 一条走线：启动键 → 连发按键（实线）或 启动键 → 停止键（虚线）。 */
export interface Wire {
  ruleId: string;
  from: string;
  to: string;
  kind: 'target' | 'stop';
  mode: BurstMode;
  enabled: boolean;
}

export interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** 横版画不出来的「高级」关系：启动键 ≠ 连发按键、切换连发的独立停止键。 */
export function buildWires(rules: WireRule[]): Wire[] {
  const wires: Wire[] = [];
  for (const r of rules) {
    const from = keyToken(r.trigger_key);
    const base = { ruleId: r.id, from, mode: r.mode, enabled: r.enabled };
    for (const key of outputKeys(r)) {
      const target = keyToken(key);
      if (target !== from) wires.push({ ...base, to: target, kind: 'target' });
    }
    if (r.mode === 'toggle' && r.stop_key) {
      const stop = keyToken(r.stop_key);
      if (stop !== from) wires.push({ ...base, to: stop, kind: 'stop' });
    }
  }
  return wires;
}

/** 悬停会画线的键：任一走线的端点。 */
export function wiredKeys(wires: Wire[]): Set<string> {
  const keys = new Set<string>();
  for (const w of wires) {
    keys.add(w.from);
    keys.add(w.to);
  }
  return keys;
}

/** 拐角切角长度：45° 斜边让走线读起来像 PCB 排线而不是表格线。 */
const CHAMFER = 6;
/** 并行走线的间距：同侧多条的通道、共用端点的进出点都按它错开。 */
export const FAN_GAP = 3.5;
/** 同一行走线的通道离键帽边沿的距离；更外层的每层再加 LANE_STEP。 */
const LANE_ABOVE = 8;
const LANE_STEP = 4;
/** 跨行走线的通道放在行缝中线上；键盘行距 5px。 */
const ROW_GAP_MID = 2.5;
/**
 * 同侧出线点从键帽外角往中心排：第一条离角 EXIT_INSET，之后每条再往里 EXIT_STEP，
 * 排不下时步长按可用宽度压缩。上沿往右那一半要躲开右上角的 ⚠ 角标（伸进键帽约 10px），
 * 离角改用 EXIT_INSET_BADGE。步长要大于端点焊盘直径，否则焊盘叠在一起。
 */
const EXIT_INSET = 4;
const EXIT_INSET_BADGE = 12;
const EXIT_STEP = 6;
/**
 * 两条平行段间距小于它就算重叠：线宽 2px 加光晕，再近就分不出是两条。
 * 不低于通道层距 LANE_STEP 与 FAN_GAP，正常的嵌套不会被误判。
 */
const MIN_PARALLEL_GAP = 3.5;
/** 可选上 / 下的走线超过这么多条就不穷举，按默认侧（出上入下）摆。 */
const MAX_CHOICES = 10;

type Point = [number, number];
type Side = 'top' | 'bottom';

/** 以悬停键为中心的一条走线：`o` 是另一端。几何一律从悬停键往外算，入线最后再反向。 */
interface Leg {
  wire: Wire;
  dir: 'out' | 'in';
  h: Rect;
  o: Rect;
  /** 另一端的键标识；同一个键的 Rect 每次量出来是新对象，归组只能按它。 */
  other: string;
  /** 与悬停键同一行，可走上方或下方。 */
  sameRow: boolean;
}

const centerX = (r: Rect) => r.left + r.width / 2;

function isSameRow(a: Rect, b: Rect): boolean {
  return Math.abs(a.top - b.top) < Math.min(a.height, b.height);
}

/** 并行组内第 i 条（共 n 条）的扇出偏移，居中对称。 */
export function fanOffset(i: number, n: number): number {
  return (i - (n - 1) / 2) * FAN_GAP;
}

/**
 * 上 / 下侧的走线：竖 → 横 → 竖。同侧同向的几条要「嵌套」才不交叉，规则是：
 * - 回到本行的（同行）按跨度由近到远逐层往外：近的通道贴键帽、出线点靠外角，
 *   远的通道更外、出线点更靠中心，像括号一层套一层。
 * - 去别的行的（跨行）末段要穿过所有通道，只能反过来：跨度远的反而在最内层，
 *   否则它的横段会切过近的那条的末段。跨行的整体排在同行的内侧。
 */
function routeVertical(legs: Leg[], side: 'top' | 'bottom'): Map<Leg, Point[]> {
  const out = new Map<Leg, Point[]>();
  const s = side === 'top' ? -1 : 1;
  const edge = (r: Rect, facingOut: boolean) =>
    (side === 'top') === facingOut ? r.top : r.top + r.height;

  for (const d of [-1, 0, 1]) {
    const group = legs.filter((l) => {
      const dx = centerX(l.o) - centerX(l.h);
      return d === 0 ? Math.abs(dx) < 1 : Math.abs(dx) >= 1 && Math.sign(dx) === d;
    });
    if (group.length === 0) continue;
    const extent = (l: Leg) => Math.abs(centerX(l.o) - centerX(l.h));
    const cross = group.filter((l) => !l.sameRow).sort((a, b) => extent(b) - extent(a));
    const same = group.filter((l) => l.sameRow).sort((a, b) => extent(a) - extent(b));
    const ordered = [...cross, ...same];

    // 正对上 / 下方的跨行键：直上直下，多条横向扇出。
    if (d === 0) {
      ordered.forEach((leg, i) => {
        const x = centerX(leg.h) + fanOffset(i, ordered.length);
        out.set(leg, [
          [x, edge(leg.h, true)],
          [x, edge(leg.o, false)],
        ]);
      });
      continue;
    }

    let crossOuter = 0;
    const dist = new Map<Leg, number>();
    cross.forEach((leg, c) => {
      const gap = Math.abs(edge(leg.h, true) - edge(leg.o, false));
      const v = Math.min(ROW_GAP_MID, gap / 2) + c * FAN_GAP;
      dist.set(leg, v);
      crossOuter = v;
    });
    const sameBase = cross.length ? Math.max(LANE_ABOVE, crossOuter + LANE_STEP) : LANE_ABOVE;
    same.forEach((leg, j) => dist.set(leg, sameBase + j * LANE_STEP));

    // 共用另一端的几条（如目标键与停止键是同一个键）在那头也要错开，外层的落点更远。
    const entryOffset = new Map<Leg, number>();
    const byOther = new Map<string, Leg[]>();
    for (const leg of ordered) byOther.set(leg.other, [...(byOther.get(leg.other) ?? []), leg]);
    for (const shared of byOther.values()) {
      shared.forEach((leg, rank) => {
        const sign = leg.sameRow ? d : -d;
        entryOffset.set(leg, sign * fanOffset(rank, shared.length));
      });
    }

    const inset = side === 'top' && d === 1 ? EXIT_INSET_BADGE : EXIT_INSET;
    ordered.forEach((leg, k) => {
      const { h, o } = leg;
      const outer = h.width / 2 - inset;
      const step = ordered.length > 1 ? Math.min(EXIT_STEP, (outer - 1) / (ordered.length - 1)) : 0;
      const xe = centerX(h) + d * (outer - k * step);
      const xo = centerX(o) + (entryOffset.get(leg) ?? 0);
      const hy = edge(h, true);
      const laneY = hy + s * (dist.get(leg) ?? LANE_ABOVE);
      const oy = leg.sameRow ? edge(o, true) : edge(o, false);
      out.set(leg, [
        [xe, hy],
        [xe, laneY],
        [xo, laneY],
        [xo, oy],
      ]);
    });
  }
  return out;
}

function route(legs: Leg[], sides: Map<Leg, Side>): Map<Leg, Point[]> {
  const pick = (side: Side) => legs.filter((l) => sides.get(l) === side);
  return new Map([
    ...routeVertical(pick('top'), 'top'),
    ...routeVertical(pick('bottom'), 'bottom'),
  ]);
}

const EPS = 0.5;

/** 两条正交折线的交叉 / 重叠次数。端点相接不算；平行段贴得比 MIN_PARALLEL_GAP 近也算重叠。 */
export function countCrossings(a: Point[], b: Point[]): number {
  let n = 0;
  for (let i = 1; i < a.length; i++) {
    for (let j = 1; j < b.length; j++) {
      if (segmentsMeet(a[i - 1], a[i], b[j - 1], b[j])) n++;
    }
  }
  return n;
}

function segmentsMeet(p1: Point, p2: Point, q1: Point, q2: Point): boolean {
  const ph = Math.abs(p1[1] - p2[1]) < EPS;
  const qh = Math.abs(q1[1] - q2[1]) < EPS;
  const inside = (v: number, a: number, b: number) =>
    v > Math.min(a, b) + EPS && v < Math.max(a, b) - EPS;
  const overlap = (a1: number, a2: number, b1: number, b2: number) =>
    Math.min(Math.max(a1, a2), Math.max(b1, b2)) - Math.max(Math.min(a1, a2), Math.min(b1, b2)) >
    EPS;
  if (ph && qh) {
    return Math.abs(p1[1] - q1[1]) < MIN_PARALLEL_GAP && overlap(p1[0], p2[0], q1[0], q2[0]);
  }
  if (!ph && !qh) {
    return Math.abs(p1[0] - q1[0]) < MIN_PARALLEL_GAP && overlap(p1[1], p2[1], q1[1], q2[1]);
  }
  const [h1, h2, v1, v2] = ph ? [p1, p2, q1, q2] : [q1, q2, p1, p2];
  return inside(v1[0], h1[0], h2[0]) && inside(h1[1], v1[1], v2[1]);
}

function totalCrossings(routes: Map<Leg, Point[]>): number {
  const all = [...routes.values()];
  let n = 0;
  for (let i = 0; i < all.length; i++) {
    for (let j = i + 1; j < all.length; j++) n += countCrossings(all[i], all[j]);
  }
  return n;
}

/**
 * 给每条走线定出线侧：跨行的走朝向对方那一侧，同行的在上 / 下里挑。
 * 不从左右侧边直连相邻键：键距只有 5px，侧边短线看不清，也和其余走线的形态不统一。
 * 同行的穷举所有组合取交叉最少的；同样少时优先出线在上、入线在下，颜色按上下分开好读。
 */
function chooseSides(legs: Leg[]): Map<Leg, Point[]> {
  const sides = new Map<Leg, Side>();
  const choices: Leg[] = [];
  for (const leg of legs) {
    if (!leg.sameRow) sides.set(leg, leg.o.top < leg.h.top ? 'top' : 'bottom');
    else {
      sides.set(leg, leg.dir === 'out' ? 'top' : 'bottom');
      choices.push(leg);
    }
  }

  let best = route(legs, sides);
  if (choices.length === 0 || choices.length > MAX_CHOICES) return best;
  let bestScore = Infinity;
  for (let mask = 0; mask < 1 << choices.length; mask++) {
    choices.forEach((leg, i) => {
      const flip = (mask >> i) & 1;
      const preferred = leg.dir === 'out' ? 'top' : 'bottom';
      const other = preferred === 'top' ? 'bottom' : 'top';
      sides.set(leg, flip ? other : preferred);
    });
    const routes = route(legs, sides);
    const flips = choices.reduce((acc, _, i) => acc + ((mask >> i) & 1), 0);
    const score = totalCrossings(routes) * 100 + flips;
    if (score < bestScore) {
      bestScore = score;
      best = routes;
    }
  }
  return best;
}

/** 把折线转成 SVG path，拐角按 CHAMFER 切成 45° 斜边（段太短时切角随之缩小）。 */
export function chamferPath(points: Point[]): string {
  if (points.length === 0) return '';
  const [x0, y0] = points[0];
  let d = `M${fmt(x0)} ${fmt(y0)}`;
  for (let i = 1; i < points.length - 1; i++) {
    const [px, py] = points[i - 1];
    const [cx, cy] = points[i];
    const [nx, ny] = points[i + 1];
    const inLen = Math.hypot(cx - px, cy - py);
    const outLen = Math.hypot(nx - cx, ny - cy);
    const c = Math.min(CHAMFER, inLen / 2, outLen / 2);
    if (c <= 0) continue;
    const ax = cx - ((cx - px) / inLen) * c;
    const ay = cy - ((cy - py) / inLen) * c;
    const bx = cx + ((nx - cx) / outLen) * c;
    const by = cy + ((ny - cy) / outLen) * c;
    d += ` L${fmt(ax)} ${fmt(ay)} L${fmt(bx)} ${fmt(by)}`;
  }
  const [lx, ly] = points[points.length - 1];
  return `${d} L${fmt(lx)} ${fmt(ly)}`;
}

export interface PlacedWire {
  id: string;
  d: string;
  /** 折线拐点（切角前），起点在启动键一端。 */
  points: Point[];
  start: Point;
  end: Point;
  dir: 'out' | 'in';
  wire: Wire;
}

/**
 * 悬停 `hoverKey` 时要画的走线：从它出去的（out）与指向它的（in）。`rectOf` 取不到的端点
 * （不可绑定、当前后端不支持的键）不画。出线侧与通道层次见 `chooseSides` / `routeVertical`。
 */
export function placeWires(
  wires: Wire[],
  hoverKey: string,
  rectOf: (token: string) => Rect | undefined,
): PlacedWire[] {
  const h = rectOf(hoverKey);
  if (!h) return [];
  const legs: Leg[] = [];
  const collect = (list: Wire[], dir: 'out' | 'in') => {
    list
      .map((wire) => {
        const other = dir === 'out' ? wire.to : wire.from;
        return { wire, other, o: rectOf(other) };
      })
      .filter((p): p is { wire: Wire; other: string; o: Rect } => !!p.o)
      .sort((a, b) => a.o.left - b.o.left)
      .forEach(({ wire, other, o }) => {
        legs.push({ wire, dir, h, o, other, sameRow: isSameRow(h, o) });
      });
  };
  collect(
    wires.filter((w) => w.from === hoverKey),
    'out',
  );
  collect(
    wires.filter((w) => w.to === hoverKey && w.from !== hoverKey),
    'in',
  );

  const routes = chooseSides(legs);
  return legs.map((leg) => {
    const fromHover = routes.get(leg) ?? [];
    const points = leg.dir === 'out' ? fromHover : [...fromHover].reverse();
    return {
      id: `${leg.dir}-${leg.wire.ruleId}-${leg.wire.kind}-${leg.wire.to}`,
      d: chamferPath(points),
      points,
      start: points[0],
      end: points[points.length - 1],
      dir: leg.dir,
      wire: leg.wire,
    };
  });
}

function fmt(v: number): string {
  return Number.isInteger(v) ? String(v) : v.toFixed(1);
}
