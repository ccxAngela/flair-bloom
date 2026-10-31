import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  availableMonitors,
  getCurrentWindow,
  LogicalSize,
  PhysicalPosition,
} from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { LazyStore } from '@tauri-apps/plugin-store';
import iconUrl from '../../assets/icon-64.png';
import { keyLabel, type KeyId } from '../panel/components/KeyCapture';
import { useKeyRelay } from '../panel/useKeyRelay';
import { useRuleStates } from '../panel/useRuleStates';
import {
  applyThemeColor,
  applyThemeMode,
  DEFAULT_THEME_COLOR,
  DEFAULT_THEME_MODE,
  type ThemeMode,
  type ThemeSettings,
} from '../panel/theme';
import {
  PANEL_LAYOUT_KEY,
  PANEL_SCALE_EVENT,
  PANEL_SCALE_KEY,
  sanitizeScales,
} from '../panel/windowScale';
import '../panel/HorizontalLayout.css';
import './FloatApp.css';

const settingsStore = new LazyStore('settings.json');
const THEME_KEY = 'theme';
const FLOAT_POS_KEY = 'float_position';

interface FloatRule {
  id: string;
  trigger_key: KeyId;
  mode: 'hold' | 'toggle';
  enabled: boolean;
  group: string | null;
}

/** 仅取浮窗渲染所需字段（结构兼容后端 BurstRule）。 */
interface BurstRuleLike {
  id: string;
  trigger_key: KeyId;
  mode: 'hold' | 'toggle';
  enabled: boolean;
  group: string | null;
}

function capClass(r: FloatRule, active: boolean, paused: boolean): string {
  return [
    'hkb-cap',
    'hkb-cap--key',
    r.enabled ? (r.mode === 'toggle' ? 'is-toggle' : 'is-hold') : 'is-off',
    active && 'is-active',
    paused && 'is-paused',
  ]
    .filter(Boolean)
    .join(' ');
}

export default function FloatApp() {
  const [active, setActive] = useState(false);
  const [globalEnabled, setGlobalEnabled] = useState(false);
  const [togglingGlobal, setTogglingGlobal] = useState(false);
  const [rules, setRules] = useState<FloatRule[]>([]);
  const floatRef = useRef<HTMLDivElement>(null);

  // 浮窗聚焦时全局键盘钩子失效，与主面板共用键盘事件中继，避免热键被吞。
  useKeyRelay();

  // ── 主题：与主面板同源（settings.json 的 theme），并跟随系统配色 ──
  const themeModeRef = useRef<ThemeMode>(DEFAULT_THEME_MODE);
  useEffect(() => {
    settingsStore
      .get<ThemeSettings>(THEME_KEY)
      .then((v) => {
        const color = v?.color ?? DEFAULT_THEME_COLOR;
        const mode = v?.mode ?? DEFAULT_THEME_MODE;
        themeModeRef.current = mode;
        applyThemeColor(color);
        applyThemeMode(mode);
      })
      .catch(() => {});

    const mql = window.matchMedia('(prefers-color-scheme: dark)');
    const onSystemChange = () => {
      if (themeModeRef.current === 'system') applyThemeMode('system');
    };
    mql.addEventListener('change', onSystemChange);

    // 主面板改主题时广播 theme-changed，浮窗实时同步（否则常驻浮窗会停留在旧主题）。
    const unlistenT = listen<ThemeSettings>('theme-changed', (e) => {
      const v = e.payload;
      themeModeRef.current = v.mode;
      applyThemeColor(v.color);
      applyThemeMode(v.mode);
    });
    return () => {
      mql.removeEventListener('change', onSystemChange);
      unlistenT.then((fn) => fn()).catch(() => {});
    };
  }, []);

  // ── 全局开关：驱动背景渐变与呼吸动画（与主面板一致）──
  useEffect(() => {
    invoke<boolean>('get_global_enabled')
      .then(setGlobalEnabled)
      .catch(() => {});
    const unlistenG = listen<boolean>('global-enabled-changed', (e) => setGlobalEnabled(e.payload));
    return () => {
      unlistenG.then((fn) => fn()).catch(() => {});
    };
  }, []);

  // ── 位置：启动恢复上次位置，拖动后持久化 ──
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let saveTimer: ReturnType<typeof setTimeout> | undefined;

    void (async () => {
      try {
        const pos = await settingsStore.get<{ x: number; y: number }>(FLOAT_POS_KEY);
        if (!pos || !Number.isFinite(pos.x) || !Number.isFinite(pos.y)) return;
        // 仅当坐标落在当前某显示器范围内才恢复，避免副屏拔除/改分辨率后浮窗停在屏幕外不可达。
        const monitors = await availableMonitors();
        const onScreen = monitors.some(
          (m) =>
            pos.x >= m.position.x &&
            pos.x < m.position.x + m.size.width &&
            pos.y >= m.position.y &&
            pos.y < m.position.y + m.size.height,
        );
        if (onScreen) {
          await getCurrentWindow().setPosition(
            new PhysicalPosition(Math.round(pos.x), Math.round(pos.y)),
          );
        }
      } catch {
        /* 恢复失败保持默认位置 */
      }
    })();

    getCurrentWindow()
      .onMoved(({ payload }) => {
        if (saveTimer) clearTimeout(saveTimer);
        saveTimer = setTimeout(() => {
          settingsStore.set(FLOAT_POS_KEY, { x: payload.x, y: payload.y }).catch(() => {});
          settingsStore.save().catch(() => {});
        }, 400);
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});

    return () => {
      if (saveTimer) clearTimeout(saveTimer);
      unlisten?.();
    };
  }, []);

  // ── 窗口宽高自适应内容：内容 hug，窗口随之收缩；比例跟随面板当前布局 ──
  useEffect(() => {
    const el = floatRef.current;
    if (!el) return;
    let zoom = 1;
    let sizeRevision = 0;
    let disposed = false;
    // 内容尺寸是 CSS 像素，webview 放大后窗口要按同一比例放大才装得下
    const apply = () => {
      const revision = ++sizeRevision;
      const rect = el.getBoundingClientRect();
      const w = Math.ceil(rect.width * zoom);
      const h = Math.ceil(rect.height * zoom);
      if (w > 0 && h > 0) {
        const win = getCurrentWindow();
        Promise.all([win.innerSize(), win.scaleFactor()])
          .then(([actual, dpi]) => {
            if (disposed || revision !== sizeRevision) return;
            if (actual.width === Math.round(w * dpi) && actual.height === Math.round(h * dpi))
              return;
            return win.setSize(new LogicalSize(w, h));
          })
          .catch(() => {});
      }
    };
    // 缩放不改内容的 CSS 尺寸，ResizeObserver 不会触发，这里手动重算一次
    const applyZoom = (next: number) => {
      zoom = next;
      getCurrentWebview()
        .setZoom(next)
        .then(apply)
        .catch(() => {});
    };
    const ro = new ResizeObserver(apply);
    ro.observe(el);
    // 系统字体度量变化可改变原生客户区，内容 CSS 尺寸不变时 ResizeObserver 不会通知。
    window.addEventListener('resize', apply);
    apply();

    Promise.all([
      settingsStore.get<string>(PANEL_LAYOUT_KEY),
      settingsStore.get<unknown>(PANEL_SCALE_KEY),
    ])
      .then(([layout, scales]) => {
        applyZoom(sanitizeScales(scales)[layout === 'horizontal' ? 'horizontal' : 'vertical']);
      })
      .catch(() => {});
    const unlistenP = listen<number>(PANEL_SCALE_EVENT, (e) => applyZoom(e.payload));
    return () => {
      disposed = true;
      ro.disconnect();
      window.removeEventListener('resize', apply);
      unlistenP.then((fn) => fn()).catch(() => {});
    };
  }, []);

  // ── 显隐：由 Rust 的 float-active 事件驱动，只在可见时轮询 ──
  useEffect(() => {
    const unlistenP = listen<boolean>('float-active', (e) => setActive(e.payload));
    // 兜底：挂载时主动查一次当前是否可见（防止错过事件）
    getCurrentWindow()
      .isVisible()
      .then(setActive)
      .catch(() => {});
    return () => {
      unlistenP.then((fn) => fn()).catch(() => {});
    };
  }, []);

  const refreshRules = useCallback(async () => {
    try {
      const list = await invoke<BurstRuleLike[]>('get_rules');
      setRules(
        list.map((r) => ({
          id: r.id,
          trigger_key: r.trigger_key,
          mode: r.mode,
          enabled: r.enabled,
          group: r.group,
        })),
      );
    } catch {
      /* 保留旧缓存 */
    }
  }, []);

  // 可见时刷新规则缓存；激活规则由 useRuleStates 每 150ms 轮询，停用即清空。
  useEffect(() => {
    if (active) void refreshRules();
  }, [active, refreshRules]);

  const onExpand = () => {
    invoke('show_main_panel').catch(() => {});
  };

  // 全局开关：复用主面板逻辑（set_global_enabled），乐观更新本地状态。
  const toggleGlobal = async () => {
    if (togglingGlobal) return;
    const next = !globalEnabled;
    setTogglingGlobal(true);
    try {
      await invoke('set_global_enabled', { enabled: next });
      setGlobalEnabled(next);
    } catch {
      /* 切换失败保持原状 */
    } finally {
      setTogglingGlobal(false);
    }
  };

  const { active: activeSet, paused: pausedSet } = useRuleStates(active, 150);
  // 含激活规则的互斥分组（保持规则出现顺序）
  const activeGroups: string[] = [];
  for (const r of rules) {
    if (r.group && activeSet.has(r.id) && !activeGroups.includes(r.group))
      activeGroups.push(r.group);
  }
  // 激活且无分组的规则，单独成键
  const singles = rules.filter((r) => activeSet.has(r.id) && !r.group);

  const renderCap = (r: FloatRule) => {
    const isActive = activeSet.has(r.id);
    const isPaused = pausedSet.has(r.id);
    const modeName = r.mode === 'toggle' ? '切换连发' : '长按连发';
    return (
      <span
        key={r.id}
        className={capClass(r, isActive, isPaused)}
        data-tauri-drag-region
        title={`${keyLabel(r.trigger_key)} · ${modeName}${r.enabled ? '' : '（已停用）'}${isPaused ? ' · 已暂停' : ''}`}
      >
        <span className="hkb-cap-label">{keyLabel(r.trigger_key)}</span>
      </span>
    );
  };

  const hasAny = singles.length > 0 || activeGroups.length > 0;

  return (
    <div
      ref={floatRef}
      className={`fb-float ${globalEnabled ? 'on' : 'off'}`}
      data-tauri-drag-region
    >
      <img className="fb-float-logo" src={iconUrl} alt="" data-tauri-drag-region />

      <div className="fb-float-content" data-tauri-drag-region>
        {!hasAny ? (
          <span className="fb-float-empty" data-tauri-drag-region>
            未连发
          </span>
        ) : (
          <>
            {singles.map(renderCap)}
            {activeGroups.map((g) => (
              <div
                key={g}
                className="fb-float-group"
                data-tauri-drag-region
                title={`互斥分组 · ${g}`}
              >
                <span className="fb-float-group-name">{g}</span>
                <div className="fb-float-group-caps" data-tauri-drag-region>
                  {rules.filter((r) => r.group === g).map(renderCap)}
                </div>
              </div>
            ))}
          </>
        )}
      </div>

      <button
        type="button"
        className={`fb-float-global ${globalEnabled ? 'is-on' : 'is-off'}`}
        title={globalEnabled ? '暂停全局连发' : '启动全局连发'}
        aria-label={globalEnabled ? '暂停全局连发' : '启动全局连发'}
        disabled={togglingGlobal}
        onClick={() => void toggleGlobal()}
      >
        {globalEnabled ? (
          // 运行中：显示暂停图标（双竖条），点击即停
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
            <rect x="7" y="5" width="3.5" height="14" rx="1.5" fill="currentColor" />
            <rect x="13.5" y="5" width="3.5" height="14" rx="1.5" fill="currentColor" />
          </svg>
        ) : (
          // 已停止：显示启动图标（三角），点击即开
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
            <path d="M8 5v14l11-7z" fill="currentColor" />
          </svg>
        )}
      </button>

      <button type="button" className="fb-float-expand" title="展开主面板" onClick={onExpand}>
        <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
          <path
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M9 3H5a2 2 0 0 0-2 2v4m18 0V5a2 2 0 0 0-2-2h-4M3 15v4a2 2 0 0 0 2 2h4m6 0h4a2 2 0 0 0 2-2v-4"
          />
        </svg>
      </button>
    </div>
  );
}
