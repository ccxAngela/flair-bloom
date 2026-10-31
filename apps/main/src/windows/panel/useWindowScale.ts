import { invoke } from '@tauri-apps/api/core';
import { LogicalSize, PhysicalPosition, PhysicalSize } from '@tauri-apps/api/dpi';
import { emit, listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { currentMonitor, cursorPosition, getCurrentWindow } from '@tauri-apps/api/window';
import type { LazyStore } from '@tauri-apps/plugin-store';
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  correctedWindowSize,
  fitScale,
  LAYOUT_BASE,
  maximizedRect,
  maxScale,
  PANEL_MAXIMIZED_KEY,
  PANEL_SCALE_EVENT,
  PANEL_SCALE_KEY,
  type PanelLayout,
  reachedMaximum,
  type Rect,
  RESIZE_ENDED_EVENT,
  restoreUnderCursor,
  sanitizeScales,
  scaledSize,
  type Size,
} from './windowScale.ts';

/** 拖边时每帧都有 resize，停下这么久才写一次 settings 并通知浮窗，不影响界面 */
const PERSIST_DELAY_MS = 400;

async function workAreaMax(base: Size): Promise<number> {
  const monitor = await currentMonitor();
  if (!monitor) return Number.POSITIVE_INFINITY;
  return maxScale(monitor.workArea.size.toLogical(monitor.scaleFactor), base);
}

async function outerRect(): Promise<Rect> {
  const win = getCurrentWindow();
  const [pos, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
  return { x: pos.x, y: pos.y, width: size.width, height: size.height };
}

/** 物理像素一次摆好位置与大小 */
async function placeWindow(r: Rect) {
  const win = getCurrentWindow();
  await win.setSize(new PhysicalSize(r.width, r.height));
  await win.setPosition(new PhysicalPosition(r.x, r.y));
}

/** 锁比例与最小尺寸都按布局的设计尺寸 */
async function lockLayout(base: Size) {
  await invoke('set_panel_aspect', { width: base.width, height: base.height });
  await getCurrentWindow().setMinSize(new LogicalSize(base.width, base.height));
}

/** 当前屏幕上最大化后的矩形（物理像素） */
async function maximizedTarget(base: Size): Promise<Rect | undefined> {
  const monitor = await currentMonitor();
  if (!monitor) return undefined;
  const f = monitor.scaleFactor;
  const { position, size } = monitor.workArea;
  return maximizedRect(
    { x: position.x, y: position.y, width: size.width, height: size.height },
    { width: base.width * f, height: base.height * f },
  );
}

/** 铺到当前屏幕工作区装得下的最大等比尺寸并居中；最大化期间不能拖边 */
async function maximizeWindow(base: Size) {
  const target = await maximizedTarget(base);
  if (!target) return;
  await getCurrentWindow().setResizable(false);
  await placeWindow(target);
}

/**
 * 面板窗口等比缩放：设计尺寸是最小尺寸，拖边时宽高比由后端 `set_panel_aspect` 在 `WM_SIZING`
 * 里锁住，这里只让内容按 webview 缩放跟上。竖版、横版各记一个比例，浮窗跟随当前布局的比例。
 * 用 webview 缩放而不是 CSS zoom：前者等同浏览器缩放，getBoundingClientRect 与鼠标坐标都在
 * 同一套 CSS 像素里，走线与引导气泡的定位不用改。
 *
 * 最大化不走系统的：比例锁着，铺满工作区会变形。拖边拉到最大、松手也算最大化。不管怎么进的，
 * 还原都回到 100% 并居中，所以不用记最大化前的位置；最大化期间不改记住的比例、不通知浮窗。
 */
export function useWindowScale(store: LazyStore) {
  const [scale, setScale] = useState(1);
  const [maximized, setMaximized] = useState(false);
  const scalesRef = useRef<Record<PanelLayout, number>>({ vertical: 1, horizontal: 1 });
  const layoutRef = useRef<PanelLayout>('vertical');
  const maximizedRef = useRef(false);
  const persistTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const resizeSeqRef = useRef(0);
  const loadedRef = useRef<Promise<void> | undefined>(undefined);

  const ensureLoaded = useCallback(() => {
    loadedRef.current ??= Promise.all([
      store.get<unknown>(PANEL_SCALE_KEY).then((v) => {
        scalesRef.current = sanitizeScales(v);
      }),
      store.get<unknown>(PANEL_MAXIMIZED_KEY).then((v) => {
        maximizedRef.current = v === true;
        setMaximized(maximizedRef.current);
      }),
    ])
      .then(() => {})
      .catch(() => {});
    return loadedRef.current;
  }, [store]);

  const persist = useCallback(
    (layout: PanelLayout, value: number) => {
      scalesRef.current = { ...scalesRef.current, [layout]: value };
      emit(PANEL_SCALE_EVENT, value).catch(() => {});
      store
        .set(PANEL_SCALE_KEY, scalesRef.current)
        .then(() => store.save())
        .catch(() => {});
    },
    [store],
  );

  /** 先落标志再动窗口：随之而来的 resize 事件靠它决定要不要记比例 */
  const setMaximizedFlag = useCallback(
    (value: boolean) => {
      maximizedRef.current = value;
      setMaximized(value);
      store
        .set(PANEL_MAXIMIZED_KEY, value)
        .then(() => store.save())
        .catch(() => {});
    },
    [store],
  );

  /** 按比例定尺寸并居中；内容缩放与持久化交给随之而来的 resize 事件 */
  const apply = useCallback(async (layout: PanelLayout, wanted: number) => {
    const base = LAYOUT_BASE[layout];
    const size = scaledSize(base, Math.min(wanted, await workAreaMax(base)));
    const win = getCurrentWindow();
    await lockLayout(base);
    await win.setSize(new LogicalSize(size.width, size.height));
    await win.center();
  }, []);

  /** 取消最大化回到 100%：给了目标矩形就摆到那里，否则居中 */
  const unmaximize = useCallback(
    async (target?: Rect) => {
      setMaximizedFlag(false);
      await getCurrentWindow().setResizable(true);
      if (target) await placeWindow(target);
      else await apply(layoutRef.current, 1);
    },
    [apply, setMaximizedFlag],
  );

  const maximize = useCallback(async () => {
    // 拖边最后几帧排下的保存还没到点；最大化不改记住的比例，不能让它晚到写进去
    clearTimeout(persistTimerRef.current);
    setMaximizedFlag(true);
    await maximizeWindow(LAYOUT_BASE[layoutRef.current]);
  }, [setMaximizedFlag]);

  /** 切换布局或启动时按该布局记住的比例定尺寸；最大化状态下换成该布局的最大尺寸 */
  const applyLayout = useCallback(
    async (layout: PanelLayout) => {
      await ensureLoaded();
      layoutRef.current = layout;
      try {
        if (maximizedRef.current) {
          await lockLayout(LAYOUT_BASE[layout]);
          await maximizeWindow(LAYOUT_BASE[layout]);
        } else {
          await apply(layout, scalesRef.current[layout]);
        }
      } catch {
        /* 尺寸调整失败不阻断交互 */
      }
    },
    [apply, ensureLoaded],
  );

  const resetScale = useCallback(async () => {
    try {
      if (maximizedRef.current) await unmaximize();
      else await apply(layoutRef.current, 1);
    } catch {
      /* 同上 */
    }
  }, [apply, unmaximize]);

  const toggleMaximize = useCallback(async () => {
    try {
      if (maximizedRef.current) await unmaximize();
      else await maximize();
    } catch {
      /* 同上 */
    }
  }, [maximize, unmaximize]);

  /** 最大化状态下按住标题栏拖动：先缩回 100%、鼠标仍捏在标题栏同一处，再交给系统拖动 */
  const dragFromMaximized = useCallback(async () => {
    try {
      const win = getCurrentWindow();
      const [cursor, current, factor] = await Promise.all([
        cursorPosition(),
        outerRect(),
        win.scaleFactor(),
      ]);
      const base = LAYOUT_BASE[layoutRef.current];
      const size = {
        width: Math.round(base.width * factor),
        height: Math.round(base.height * factor),
      };
      await unmaximize(restoreUnderCursor(cursor, current, size));
      await win.startDragging();
    } catch {
      /* 同上 */
    }
  }, [unmaximize]);

  useEffect(() => {
    const win = getCurrentWindow();
    let disposed = false;
    const synchronize = async (payload: PhysicalSize) => {
      if (disposed) return;
      // 拖边每帧一个事件，中间要等两次 IPC，先发的可能后到；只认最新一帧
      const seq = ++resizeSeqRef.current;
      const factor = await win.scaleFactor();
      const size = payload.toLogical(factor);
      const layout = layoutRef.current;
      const base = LAYOUT_BASE[layout];
      const max = await workAreaMax(base);
      if (disposed || seq !== resizeSeqRef.current) return;
      const corrected = correctedWindowSize(size, base, max);
      if (corrected) {
        await win.setSize(new LogicalSize(corrected.width, corrected.height));
        return;
      }
      // 最小化时 Windows 报 0×0；有最小尺寸兜着，低于设计尺寸的都不是用户拖出来的
      if (size.width < base.width - 1 || size.height < base.height - 1) return;
      const next = fitScale(size, base, max);
      getCurrentWebview()
        .setZoom(next)
        .catch(() => {});
      setScale(next);
      // 切布局时 setMinSize 先把窗口撑到中间尺寸，会算出一个过渡比例；去抖只留最后一次
      clearTimeout(persistTimerRef.current);
      if (maximizedRef.current) return;
      persistTimerRef.current = setTimeout(() => persist(layout, next), PERSIST_DELAY_MS);
    };
    const unlisten = win.onResized(({ payload }) => void synchronize(payload));
    // WM_SETTINGCHANGE 可只让 WebView 收到视口变化，不一定触发 Tauri 的 Resized。
    const onViewportResize = () => {
      win
        .innerSize()
        .then(synchronize)
        .catch(() => {});
    };
    window.addEventListener('resize', onViewportResize);
    return () => {
      disposed = true;
      resizeSeqRef.current += 1;
      clearTimeout(persistTimerRef.current);
      window.removeEventListener('resize', onViewportResize);
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [persist]);

  // 拖边拉到最大、松手即转成最大化。只在松手时判断：拖动中途禁止拖边会把正在进行的拖动掐断
  useEffect(() => {
    const unlisten = listen(RESIZE_ENDED_EVENT, async () => {
      if (maximizedRef.current) return;
      try {
        const win = getCurrentWindow();
        const base = LAYOUT_BASE[layoutRef.current];
        const [target, current, factor] = await Promise.all([
          maximizedTarget(base),
          win.outerSize(),
          win.scaleFactor(),
        ]);
        const physicalBase = { width: base.width * factor, height: base.height * factor };
        if (target && reachedMaximum(current, target, physicalBase)) await maximize();
      } catch {
        /* 同上 */
      }
    });
    return () => {
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [maximize]);

  return { scale, maximized, applyLayout, resetScale, toggleMaximize, dragFromMaximized };
}
