import { outputKeys, type OutputStep } from './outputSequence.ts';
/**
 * 热键冲突检测。
 *
 * 规则：
 *  - ERROR  功能损坏：面板键遮蔽全局键；多条已启用规则共用同一触发键
 *  - WARNING 行为意外：全局热键与规则触发键重叠（热键优先返回，该按键不触发连发）
 *
 * 不检测：
 *  - global_toggle == global_stop（有意的切换模式）
 *  - 全局热键 == target_key（target 走注入通道，hook 过滤自身注入）
 */

import { keyEq, keyLabel, type KeyId } from './components/KeyCapture';
import { keyToken } from './keyToken';

export type BurstMode = 'hold' | 'toggle';

export interface BurstRule {
  id: string;
  enabled: boolean;
  trigger_key: KeyId;
  steps: OutputStep[];
  mode: BurstMode;
}

export interface Hotkeys {
  global_toggle: KeyId | null;
  global_stop: KeyId | null;
  panel_toggle: KeyId | null;
}

export type ConflictSeverity = 'error' | 'warning';

export type ConflictParticipant =
  | { kind: 'global'; field: 'global_toggle' | 'global_stop' | 'panel_toggle'; label: string }
  | {
      kind: 'rule';
      ruleId: string;
      ruleMode: BurstMode;
      field: 'trigger_key' | 'target_key';
      label: string;
    };

export interface Conflict {
  id: string;
  severity: ConflictSeverity;
  key: KeyId;
  message: string;
  participants: ConflictParticipant[];
}

// ── 工具 ─────────────────────────────────────────────────────────────────────

const GLOBAL_LABELS: Record<string, string> = {
  global_toggle: '全局开启键',
  global_stop: '全局停止键',
  panel_toggle: '面板显隐键',
};

// ── 主算法 ────────────────────────────────────────────────────────────────────

export function detectConflicts(rules: BurstRule[], hotkeys: Hotkeys): Conflict[] {
  const conflicts: Conflict[] = [];

  // 仅检查已启用的规则
  const enabled = rules.filter((r) => r.enabled);

  // ── 1. 面板键遮蔽全局开启键 ────────────────────────────────────────────────
  if (keyEq(hotkeys.panel_toggle, hotkeys.global_toggle)) {
    conflicts.push({
      id: 'panel-shadows-toggle',
      severity: 'error',
      key: hotkeys.panel_toggle!,
      message: '面板显隐键与全局开启键相同，面板热键优先处理，全局开启键将失效',
      participants: [
        { kind: 'global', field: 'panel_toggle', label: GLOBAL_LABELS.panel_toggle },
        { kind: 'global', field: 'global_toggle', label: GLOBAL_LABELS.global_toggle },
      ],
    });
  }

  // ── 2. 面板键遮蔽全局停止键（且两者不同，避免与上条重复）──────────────────
  // global_stop 为 null 时有效值等同 global_toggle（切换模式），不需要单独检测
  if (
    hotkeys.global_stop &&
    !keyEq(hotkeys.global_stop, hotkeys.global_toggle) &&
    keyEq(hotkeys.panel_toggle, hotkeys.global_stop)
  ) {
    conflicts.push({
      id: 'panel-shadows-stop',
      severity: 'error',
      key: hotkeys.panel_toggle!,
      message: '面板显隐键与全局停止键相同，面板热键优先处理，全局停止键将失效',
      participants: [
        { kind: 'global', field: 'panel_toggle', label: GLOBAL_LABELS.panel_toggle },
        { kind: 'global', field: 'global_stop', label: GLOBAL_LABELS.global_stop },
      ],
    });
  }

  // ── 3. 全局热键与规则触发键重叠（warning）──────────────────────────────────
  // 构建需要检查的全局键列表（去重：toggle==stop 不重复报）
  const globalKeys: Array<{
    key: KeyId;
    field: 'global_toggle' | 'global_stop' | 'panel_toggle';
  }> = [];
  if (hotkeys.global_toggle) {
    globalKeys.push({ key: hotkeys.global_toggle, field: 'global_toggle' });
  }
  if (hotkeys.global_stop && !keyEq(hotkeys.global_stop, hotkeys.global_toggle)) {
    globalKeys.push({ key: hotkeys.global_stop, field: 'global_stop' });
  }
  if (hotkeys.panel_toggle) {
    globalKeys.push({ key: hotkeys.panel_toggle, field: 'panel_toggle' });
  }

  for (const { key, field } of globalKeys) {
    const clashing = enabled.filter((r) => keyEq(key, r.trigger_key));
    if (clashing.length === 0) continue;

    // 若该 key 已在第 1/2 条（ERROR）里出现，不再重复 warning
    const alreadyError = conflicts.some((c) => c.severity === 'error' && keyEq(c.key, key));
    if (alreadyError) continue;

    conflicts.push({
      id: `hotkey-masks-rule-${field}-${keyToken(key)}`,
      severity: 'warning',
      key,
      message: `${GLOBAL_LABELS[field]}与规则触发键相同，按下时热键优先，该按键不触发连发`,
      participants: [
        { kind: 'global', field, label: GLOBAL_LABELS[field] },
        ...clashing.map((r) => ({
          kind: 'rule' as const,
          ruleId: r.id,
          ruleMode: r.mode,
          field: 'trigger_key' as const,
          label: r.mode === 'hold' ? '长按连发' : '切换连发',
        })),
      ],
    });
  }

  // ── 4. 连发按键（target_key）== 另一条规则的触发键（warning）──────────────
  // 注入的 target_key 事件虽由 SIM_MARKER / PENDING_INJECTIONS 过滤，
  // 但 DD-HID 模式在极端情况下可能漏判，配置本身在概念上也令人困惑。
  for (const original of enabled) {
    for (const key of outputKeys(original)) {
      const ruleA = { ...original, steps: [{ key, interval_ms: 10 }] };
      const cascading = enabled.filter(
        (ruleB) => ruleB.id !== ruleA.id && keyEq(ruleA.steps[0].key, ruleB.trigger_key),
      );
      if (cascading.length === 0) continue;

      // 若该 key 已被更高优先级冲突覆盖，不重复报
      const alreadyCovered = conflicts.some(
        (c) =>
          keyEq(c.key, ruleA.steps[0].key) &&
          c.participants.some((p) => p.kind === 'rule' && p.ruleId === ruleA.id),
      );
      if (alreadyCovered) continue;

      conflicts.push({
        id: `target-cascades-trigger-${ruleA.id}-${keyToken(ruleA.steps[0].key)}`,
        severity: 'warning',
        key: ruleA.steps[0].key,
        message: `连发按键 ${keyLabel(ruleA.steps[0].key)} 同时是另一条规则的触发键，注入时可能意外激活该规则`,
        participants: [
          {
            kind: 'rule',
            ruleId: ruleA.id,
            ruleMode: ruleA.mode,
            field: 'target_key',
            label: '连发来源',
          },
          ...cascading.map((r) => ({
            kind: 'rule' as const,
            ruleId: r.id,
            ruleMode: r.mode,
            field: 'trigger_key' as const,
            label: '被触发',
          })),
        ],
      });
    }
  }
  return conflicts;
}

// ── 查询工具（供 UI 组件高亮具体控件）────────────────────────────────────────

/** 给定 KeyId，返回涉及它的最高冲突级别（用于 KeyCapture 控件着色）。 */
export function severityForKey(conflicts: Conflict[], key: KeyId | null): ConflictSeverity | null {
  if (!key) return null;
  const matching = conflicts.filter((c) => keyEq(c.key, key));
  if (matching.some((c) => c.severity === 'error')) return 'error';
  if (matching.some((c) => c.severity === 'warning')) return 'warning';
  return null;
}

/** 给定规则 ID，返回涉及该规则的最高冲突级别。 */
export function severityForRule(conflicts: Conflict[], ruleId: string): ConflictSeverity | null {
  const matching = conflicts.filter((c) =>
    c.participants.some((p) => p.kind === 'rule' && p.ruleId === ruleId),
  );
  if (matching.some((c) => c.severity === 'error')) return 'error';
  if (matching.some((c) => c.severity === 'warning')) return 'warning';
  return null;
}
