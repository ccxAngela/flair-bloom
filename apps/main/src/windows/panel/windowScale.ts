// 面板窗口等比缩放的纯计算：设计尺寸是最小尺寸，窗口按同一比例放大，内容用 webview 缩放跟上。

export interface Size {
  width: number;
  height: number;
}

export interface Point {
  x: number;
  y: number;
}

export interface Rect extends Point, Size {}

export type PanelLayout = 'vertical' | 'horizontal';

/** settings.json 里的布局与各布局缩放比例；浮窗也读这两个键算自己的初始比例 */
export const PANEL_LAYOUT_KEY = 'layout';
export const PANEL_SCALE_KEY = 'panelScale';
/** 面板是否最大化；重开面板、重启应用都保持 */
export const PANEL_MAXIMIZED_KEY = 'panelMaximized';
/** 拖边松手时后端发给面板（`commands/window.rs` 的 `RESIZE_ENDED_EVENT`） */
export const RESIZE_ENDED_EVENT = 'panel-resize-ended';
/** 面板比例停稳后广播当前生效的比例，浮窗跟随 */
export const PANEL_SCALE_EVENT = 'panel-scale-changed';

/**
 * 两种布局的设计尺寸（逻辑像素），也是窗口最小尺寸。取常用比例：竖版 9:16、横版 16:9。
 * 横版键鼠图一屏装下不滚动，排版至少要 1060×580，16:9 取不小于它的最近尺寸。
 */
export const LAYOUT_BASE: Record<PanelLayout, Size> = {
  vertical: { width: 405, height: 720 },
  horizontal: { width: 1072, height: 603 },
};

/** 缩放比例保留两位小数：存进 settings 与显示百分比都不带浮点尾巴 */
const round2 = (n: number) => Math.round(n * 100) / 100;

/** 窗口能放下的最大比例：工作区（去掉任务栏）装得下的最大等比尺寸，至少 1 */
export function maxScale(workArea: Size, base: Size): number {
  return Math.max(
    1,
    Math.floor(Math.min(workArea.width / base.width, workArea.height / base.height) * 100) / 100,
  );
}

const clamp = (scale: number, max: number) => Math.min(max, Math.max(1, round2(scale)));

/** 内容缩放：窗口比例由后端锁住，取宽高比例里较小的只是兜住舍入误差与非 Windows 平台 */
export function fitScale(size: Size, base: Size, max: number): number {
  return clamp(Math.min(size.width / base.width, size.height / base.height), max);
}

export function scaledSize(base: Size, scale: number): Size {
  return { width: Math.round(base.width * scale), height: Math.round(base.height * scale) };
}

/** 原生字体度量变化可使无边框客户区变矮；保留宽度，把比例及最小尺寸校正回来。 */
export function correctedWindowSize(size: Size, base: Size, max: number): Size | undefined {
  if (size.width <= 0 || size.height <= 0) return undefined;
  const target = scaledSize(base, Math.min(max, Math.max(1, size.width / base.width)));
  return Math.abs(target.width - size.width) > 1 || Math.abs(target.height - size.height) > 1
    ? target
    : undefined;
}

/**
 * 最大化后的窗口矩形：工作区装得下的最大等比尺寸，在工作区里居中。比例锁着，铺满工作区会变形。
 * 坐标单位与传入的工作区一致。
 */
export function maximizedRect(workArea: Rect, base: Size): Rect {
  const size = scaledSize(base, maxScale(workArea, base));
  return {
    x: Math.round(workArea.x + (workArea.width - size.width) / 2),
    y: Math.round(workArea.y + (workArea.height - size.height) / 2),
    ...size,
  };
}

/**
 * 拖边松手时是否算拉到了最大。工作区装不下比设计尺寸更大的窗口时，最大化尺寸就是最小尺寸，
 * 随便拖一下都「到顶」，不能算。后端锁的上限按像素取整、前端按两位小数的比例取整，
 * 拉到顶时窗口只会比 target 大一点，所以用 >=。单位与传入的参数一致。
 */
export function reachedMaximum(current: Size, target: Size, base: Size): boolean {
  return target.width > base.width + 1 && current.width >= target.width;
}

/**
 * 最大化状态下拖标题栏时还原出的矩形：鼠标在窗口里的相对位置不变，像系统窗口那样
 * 「捏」着同一处标题栏缩回去。坐标单位与传入的参数一致。
 */
export function restoreUnderCursor(cursor: Point, maximized: Rect, restored: Size): Rect {
  const fx = (cursor.x - maximized.x) / maximized.width;
  const fy = (cursor.y - maximized.y) / maximized.height;
  return {
    x: Math.round(cursor.x - fx * restored.width),
    y: Math.round(cursor.y - fy * restored.height),
    ...restored,
  };
}

/** settings 里存的比例：缺失、非数或小于 1 的一律当 1，防止旧数据或手改把窗口缩到设计尺寸以下 */
export function sanitizeScales(raw: unknown): Record<PanelLayout, number> {
  const pick = (v: unknown) =>
    typeof v === 'number' && Number.isFinite(v) && v >= 1 ? round2(v) : 1;
  const obj = (raw ?? {}) as Partial<Record<PanelLayout, unknown>>;
  return { vertical: pick(obj.vertical), horizontal: pick(obj.horizontal) };
}
