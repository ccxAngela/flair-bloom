import { isSimpleRule, type OutputStep } from './outputSequence.ts';
import { type CSSProperties, useEffect, useRef, useState } from 'react';
import WireOverlay from './WireOverlay';
import { buildWires, wiredKeys } from './hkbWires';
import { keyToken as token } from './keyToken';
import ContextMenu, { type ContextMenuItem } from './components/ContextMenu';
import IntervalInput from './components/IntervalInput';
import {
  keyboardKey,
  keyLabel,
  mouseKey,
  type KeyId,
  type MouseButton,
  type SlotPolicy,
} from './components/KeyCapture';
import { type Conflict, severityForKey } from './conflicts';
import {
  type KeyCap,
  type KeyRow,
  MAIN_BLOCK,
  MOUSE_ROWS,
  NAV_BLOCK,
  NUMPAD_CELLS,
} from './keyboardLayout';
import './HorizontalLayout.css';

// 键宽单位：标准键宽 UNIT，键间距 GAP；多单位键宽度需补足跨越的间距。
const UNIT = 40;
const GAP = 5;
const span = (n = 1) => n * UNIT + (n - 1) * GAP;
const capWidth = (w = 1) => `${span(w)}px`;

/** 横版只需规则的这些字段（结构上兼容 PanelApp 的 BurstRule）。 */
export interface HRule {
  id: string;
  enabled: boolean;
  trigger_key: KeyId;
  steps: OutputStep[];
  stop_key: KeyId | null;
  mode: 'hold' | 'toggle';
  group: string | null;
}

interface Props {
  rules: HRule[];
  /**
   * 单键模型的允许集：横版每个键位都是 `trigger == target` 的重合态，故用后端下发的
   * `trigger_target` 槽。不在集合里的键只作观感、不可点，避免用户点上去再在保存时报错。
   */
  policy: SlotPolicy;
  activeRuleIds: Set<string>;
  /** 被同组长按插队而暂停的规则（仍在 activeRuleIds 里）。 */
  pausedRuleIds: Set<string>;
  conflicts: Conflict[];
  /** 统一连发间隔（所有单键规则共享）。 */
  interval: number;
  intervalMin: number;
  intervalMax: number;
  onIntervalChange: (ms: number) => void;
  /** 左键点键：在 无 → 切换 → 长按 → 无 之间轮换。 */
  onCycleKey: (key: KeyId) => void;
  onEditDetails: () => void;
  onSetEnabled: (ruleId: string, enabled: boolean) => void;
  onSetMode: (rule: HRule, mode: 'hold' | 'toggle') => void;
  onSetGroup: (ruleId: string, group: string | null) => void;
  onCreateGroupWith: (ruleId: string) => void;
  onDeleteRule: (ruleId: string) => void;
  /** 删除某物理键上的全部规则（高级规则键用）。 */
  onDeleteKey: (key: KeyId) => void;
}

/** 某物理键上的规则分类结果。 */
interface KeyState {
  rules: HRule[];
  /** 唯一一条单键规则（trigger==target）；否则 null。 */
  single: HRule | null;
  /** 高级：>1 条规则，或唯一规则 trigger≠target。 */
  advanced: boolean;
}

export default function HorizontalLayout({
  rules,
  policy,
  activeRuleIds,
  pausedRuleIds,
  conflicts,
  interval,
  intervalMin,
  intervalMax,
  onIntervalChange,
  onCycleKey,
  onEditDetails,
  onSetEnabled,
  onSetMode,
  onSetGroup,
  onCreateGroupWith,
  onDeleteRule,
  onDeleteKey,
}: Props) {
  const anchorRef = useRef<HTMLElement | null>(null);
  const [menuKey, setMenuKey] = useState<KeyId | null>(null);
  const layoutRef = useRef<HTMLElement | null>(null);
  // 键帽 DOM 按 token 登记，悬停时量两端位置画走线。
  const capRefs = useRef(new Map<string, HTMLElement>());
  const [hoverKey, setHoverKey] = useState<string | undefined>(undefined);
  // 走线几何只在悬停那一刻量一次，窗口缩放后线会与键帽错位，直接收起、重新移入再画——
  // 比实时重算省事。横版一屏放下、不滚动，所以不必管滚动。
  useEffect(() => {
    if (!hoverKey) return;
    const clear = () => setHoverKey(undefined);
    window.addEventListener('resize', clear);
    return () => window.removeEventListener('resize', clear);
  }, [hoverKey]);
  const wires = buildWires(rules);
  const wired = wiredKeys(wires);

  // 触发键 token → 规则列表。
  const byTrigger = new Map<string, HRule[]>();
  for (const r of rules) {
    const t = token(r.trigger_key);
    const list = byTrigger.get(t);
    if (list) list.push(r);
    else byTrigger.set(t, [r]);
  }

  function stateOf(key: KeyId): KeyState {
    const list = byTrigger.get(token(key)) ?? [];
    if (list.length === 0) return { rules: [], single: null, advanced: false };
    if (list.length === 1 && isSimpleRule(list[0])) {
      return { rules: list, single: list[0], advanced: false };
    }
    return { rules: list, single: null, advanced: true };
  }

  // 分组顺序 → 序号（图例与键帽角标共用）。
  const groupOrder: string[] = [];
  for (const r of rules) {
    if (r.group && isSimpleRule(r) && !groupOrder.includes(r.group)) {
      groupOrder.push(r.group);
    }
  }
  const groupIndex = (g: string) => groupOrder.indexOf(g) + 1;
  const hasAdvanced = rules.some((r) => !isSimpleRule(r));

  function buildMenu(key: KeyId, st: KeyState): ContextMenuItem[] {
    if (st.advanced) {
      return [
        { label: '在竖版编辑此键的规则', onClick: onEditDetails },
        { type: 'divider' },
        { label: '删除此键全部规则', danger: true, onClick: () => onDeleteKey(key) },
      ];
    }
    const rule = st.single;
    if (!rule) return [];
    const items: ContextMenuItem[] = [
      {
        label: rule.enabled ? '停用' : '启用',
        onClick: () => onSetEnabled(rule.id, !rule.enabled),
      },
      {
        label: rule.mode === 'toggle' ? '改为长按连发' : '改为切换连发',
        onClick: () => onSetMode(rule, rule.mode === 'toggle' ? 'hold' : 'toggle'),
      },
    ];
    const groupChildren: ContextMenuItem[] = groupOrder
      .filter((g) => g !== rule.group)
      .map((g) => ({ label: g, onClick: () => onSetGroup(rule.id, g) }));
    groupChildren.push({ label: '新建互斥分组…', onClick: () => onCreateGroupWith(rule.id) });
    items.push({ label: '移入互斥分组', children: groupChildren });
    if (rule.group) {
      items.push({ label: `移出「${rule.group}」`, onClick: () => onSetGroup(rule.id, null) });
    }
    items.push({ type: 'divider' });
    items.push({ label: '删除', danger: true, onClick: () => onDeleteRule(rule.id) });
    return items;
  }

  /** 渲染一个可绑定键帽（键盘 / 鼠标共用）。 */
  function bindableCap(
    reactKey: string | number,
    key: KeyId,
    label: string,
    style: CSSProperties,
    extraClass = '',
  ) {
    const st = stateOf(key);
    const rule = st.single;
    const active = rule ? activeRuleIds.has(rule.id) : false;
    const paused = rule ? pausedRuleIds.has(rule.id) : false;
    const severity = severityForKey(conflicts, key);
    const warn = st.advanced || severity !== null;
    const gi = rule?.group ? groupIndex(rule.group) : 0;
    const modeClass = rule ? (rule.mode === 'toggle' ? 'is-toggle' : 'is-hold') : '';
    const t = token(key);
    const hoverProps = wired.has(t)
      ? {
          onMouseEnter: () => setHoverKey(t),
          onMouseLeave: () => setHoverKey((k) => (k === t ? undefined : k)),
          onFocus: () => setHoverKey(t),
          onBlur: () => setHoverKey((k) => (k === t ? undefined : k)),
        }
      : {};

    const cls = [
      'hkb-cap',
      'hkb-cap--key',
      extraClass,
      rule && (rule.enabled ? modeClass : 'is-off'),
      active && 'is-active',
      paused && 'is-paused',
      st.advanced && 'is-advanced',
      gi > 0 && 'is-grouped',
      severity === 'error' && 'is-error',
      severity === 'warning' && 'is-warn',
    ]
      .filter(Boolean)
      .join(' ');

    let title: string;
    if (st.advanced) {
      title = `${keyLabel(key)} · 高级规则（仅竖版可编辑）`;
    } else if (rule) {
      const modeName = rule.mode === 'toggle' ? '切换连发' : '长按连发';
      title = `${keyLabel(key)} · ${modeName}${rule.enabled ? '' : '（已停用）'}${rule.group ? ` · ${rule.group}` : ''}${paused ? ' · 已暂停（同组长按插队中）' : ''}（左键轮换 · 右键管理）`;
    } else {
      title = `${keyLabel(key)}（点击建立连发）`;
    }

    return (
      <button
        key={reactKey}
        ref={(el) => {
          if (el) capRefs.current.set(t, el);
          else capRefs.current.delete(t);
        }}
        type="button"
        className={cls}
        style={style}
        title={title}
        {...hoverProps}
        onClick={() => {
          if (st.advanced) onEditDetails();
          else onCycleKey(key);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          if (st.rules.length > 0) {
            anchorRef.current = e.currentTarget;
            setMenuKey(key);
          }
        }}
      >
        <span className="hkb-cap-label">{label}</span>
        {warn && (
          <span className="hkb-warn-badge" aria-hidden="true">
            ⚠️
          </span>
        )}
        {gi > 0 && rule && (
          <span className={`hkb-group-badge ${rule.mode === 'toggle' ? 'is-toggle' : 'is-hold'}`}>
            {gi}
          </span>
        )}
      </button>
    );
  }

  function renderCap(cap: KeyCap, idx: number) {
    const style: CSSProperties = { width: capWidth(cap.w) };
    if (cap.spacer || cap.vk === undefined) {
      if (cap.spacer || cap.label === undefined) {
        return <span key={idx} className="hkb-spacer" style={style} />;
      }
      return (
        <span key={idx} className="hkb-cap hkb-cap--deco" style={style}>
          {cap.label}
        </span>
      );
    }
    if (cap.bindable === false) {
      return (
        <span key={idx} className="hkb-cap hkb-cap--deco" style={style}>
          {cap.label ?? keyLabel(keyboardKey(cap.vk))}
        </span>
      );
    }
    return bindableCap(idx, keyboardKey(cap.vk), cap.label ?? keyLabel(keyboardKey(cap.vk)), style);
  }

  function renderBlock(block: KeyRow[], className: string) {
    return (
      <div className={`hkb-block ${className}`} style={{ gap: `${GAP}px` }}>
        {block.map((row, ri) => (
          <div key={ri} className="hkb-row" style={{ gap: `${GAP}px` }}>
            {row.map((cap, ci) => renderCap(cap, ci))}
          </div>
        ))}
      </div>
    );
  }

  return (
    <section className="horizontal-layout" ref={layoutRef}>
      <div className="hkb-keyboard" data-tour="hkb-keyboard">
        {renderBlock(MAIN_BLOCK, 'hkb-main')}
        {renderBlock(NAV_BLOCK, 'hkb-nav')}
        <div
          className="hkb-numpad-grid"
          style={{
            gap: `${GAP}px`,
            gridTemplateColumns: `repeat(4, ${UNIT}px)`,
            gridAutoRows: `${UNIT}px`,
          }}
        >
          {NUMPAD_CELLS.map((cell, i) => {
            const gridStyle: CSSProperties = {
              gridColumn: `${cell.col} / span ${cell.colSpan ?? 1}`,
              gridRow: `${cell.row} / span ${cell.rowSpan ?? 1}`,
            };
            if (cell.bindable === false || cell.vk === undefined) {
              return (
                <span key={i} className="hkb-cap hkb-cap--deco" style={gridStyle}>
                  {cell.label}
                </span>
              );
            }
            return bindableCap(
              i,
              keyboardKey(cell.vk),
              cell.label ?? keyLabel(keyboardKey(cell.vk)),
              gridStyle,
            );
          })}
        </div>
      </div>

      <div className="hdivider" aria-hidden="true" />

      <div className="hbottom">
        {/* 左：鼠标三行 */}
        <div className="hmouse">
          <span className="hmouse-title">鼠标</span>
          <div className="hmouse-grid">
            {MOUSE_ROWS.map((row, ri) => (
              <div key={ri} className="hmouse-row">
                {row.map((m: { code: MouseButton; label: string }) =>
                  // 当前后端注入不了的按钮置灰不可点，避免用户点上去、保存时才被拒绝。
                  // 现存三个后端都覆盖全部按钮，这里是给将来受限的后端留的口子。
                  policy.mouse.includes(m.code) ? (
                    bindableCap(m.code, mouseKey(m.code), m.label, { width: '64px' }, 'hmouse-cap')
                  ) : (
                    <span
                      key={m.code}
                      className="hkb-cap hkb-cap--deco hmouse-cap"
                      style={{ width: '64px' }}
                      title="当前输入模式不支持此按键"
                    >
                      {m.label}
                    </span>
                  ),
                )}
              </div>
            ))}
          </div>
        </div>

        {/* 中：信息栏 —— 图例 / 分组列表（占剩余空间）/ 统一间隔 */}
        <div className="hbottom-center">
          <div className="hbar-legend" data-tour="hbar-legend">
            <span className="hbar-legend-item">
              <span className="hbar-swatch sw-toggle" />
              切换连发
            </span>
            <span className="hbar-legend-item">
              <span className="hbar-swatch sw-hold" />
              长按连发
            </span>
            <span className="hbar-legend-item">
              <span className="hbar-swatch sw-off" />
              已停用
            </span>
            <span
              className="hbar-legend-item"
              title="组号角标：实心 = 长按在组内，描边 = 切换在组内"
            >
              <span className="hbar-swatch sw-group">1</span>
              <span className="hbar-swatch sw-group is-toggle">1</span>
              互斥分组
            </span>
            {wires.length > 0 && (
              <span className="hbar-legend-item" title="悬停参与高级规则的键查看走线">
                <span className="hbar-wire sw-wire-out" />
                它触发的
                <span className="hbar-wire sw-wire-in" />
                触发它的
              </span>
            )}
            {hasAdvanced && (
              <span
                className="hbar-legend-item"
                title="trigger≠target / 同键多规则等，横版只读，请在竖版编辑"
              >
                <span className="hbar-warn-icon">⚠️</span>
                高级规则（仅竖版）
              </span>
            )}
          </div>

          <div className="hbar-groups">
            {groupOrder.map((g, i) => (
              <span key={g} className="hbar-group">
                <span className="hbar-group-no">{i + 1}</span>
                <span className="hbar-group-name">{g}</span>
                <span className="hbar-group-keys">
                  {rules
                    .filter((r) => r.group === g && isSimpleRule(r))
                    .map((r) => (
                      <span key={r.id} className="hbar-key-chip">
                        {keyLabel(r.trigger_key)}
                      </span>
                    ))}
                </span>
              </span>
            ))}
          </div>

          <div className="hbar-interval" data-tour="hbar-interval">
            <span>统一间隔</span>
            <IntervalInput
              value={interval}
              min={intervalMin}
              max={intervalMax}
              onChange={onIntervalChange}
            />
          </div>
        </div>

        {/* 右：空占位，与左侧鼠标对称，使中间内容居中 */}
        <div className="hbottom-spacer" aria-hidden="true" />
      </div>

      {hoverKey && (
        <WireOverlay
          hoverKey={hoverKey}
          wires={wires}
          container={layoutRef.current}
          caps={capRefs.current}
        />
      )}
      <ContextMenu
        open={menuKey !== null}
        onClose={() => setMenuKey(null)}
        target={anchorRef}
        items={menuKey ? buildMenu(menuKey, stateOf(menuKey)) : []}
        location="bottom-left"
      />
    </section>
  );
}
