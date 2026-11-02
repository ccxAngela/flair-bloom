import { useRef, useState, type DragEventHandler } from 'react';
import ContextMenu, { type ContextMenuItem } from './components/ContextMenu';
import { ChevronIcon } from './components/icons';
import IntervalInput from './components/IntervalInput';
import KeyCapture, {
  type CaptureReject,
  keyEq,
  type KeyId,
  type KeyPolicies,
} from './components/KeyCapture';
import type { ConflictSeverity } from './conflicts';
import SequenceEditor from './components/SequenceEditor';
import type { OutputStep } from './outputSequence';
import type { RuleProgress } from './useRuleStates';

type BurstMode = 'hold' | 'toggle';

/** 与 PanelApp 的 BurstRule 结构兼容，不反向依赖它。 */
export interface CardRule {
  id: string;
  enabled: boolean;
  trigger_key: KeyId;
  steps: OutputStep[];
  mode: BurstMode;
  stop_key: KeyId | null;
  group: string | null;
}

export interface CardDragHandlers {
  onDragStart: DragEventHandler<HTMLDivElement>;
  onDragOver: DragEventHandler<HTMLDivElement>;
  onDrop: DragEventHandler<HTMLDivElement>;
  onDragEnd: DragEventHandler<HTMLDivElement>;
}

interface Props {
  rule: CardRule;
  progress?: RuleProgress;
  isActive: boolean;
  isPaused: boolean;
  /** 教程判定的「最后一条可见规则」（`lastRule`），承载教程锚点 `rule-latest`。 */
  isLatest: boolean;
  showAdvanced: boolean;
  isDragging: boolean;
  isDragTarget: boolean;
  conflict: ConflictSeverity | null;
  policies: KeyPolicies;
  intervalMin: number;
  intervalMax: number;
  /** 全部已有分组名，供「移入分组」子菜单。 */
  groups: string[];
  drag: CardDragHandlers;
  onPatch: (patch: Partial<CardRule>) => void;
  onSaveSequence: (steps: OutputStep[]) => Promise<void>;
  onSetMode: (mode: BurstMode) => void;
  onToggleAdvanced: () => void;
  onDelete: () => void;
  onMoveToGroup: (group: string) => void;
  onNewGroupWith: () => void;
  onRemoveFromGroup: () => void;
  onKeyReject: (info: CaptureReject) => void;
  /** 成功录入一次连发按键（教程实操判定用）。 */
  onTargetCaptured: () => void;
}

const MODE_NAME: Record<BurstMode, string> = { hold: '长按', toggle: '切换' };

function other(mode: BurstMode): BurstMode {
  return mode === 'hold' ? 'toggle' : 'hold';
}

export default function RuleCard({
  rule,
  progress,
  isActive,
  isPaused,
  isLatest,
  showAdvanced,
  isDragging,
  isDragTarget,
  conflict,
  policies,
  intervalMin,
  intervalMax,
  groups,
  drag,
  onPatch,
  onSaveSequence,
  onSetMode,
  onToggleAdvanced,
  onDelete,
  onMoveToGroup,
  onNewGroupWith,
  onRemoveFromGroup,
  onKeyReject,
  onTargetCaptured,
}: Props) {
  const menuRef = useRef<HTMLButtonElement>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [sequenceOpen, setSequenceOpen] = useState(false);
  const multiple = rule.steps.length > 1;
  const isToggle = rule.mode === 'toggle';
  // 长按键与连发键分开时把「按住哪个键」提到主行：插队要按住的就是它，不能藏在高级设置里。
  const holdSplit = !isToggle && (multiple || !keyEq(rule.trigger_key, rule.steps[0].key));
  const showTrigger = isToggle || holdSplit;
  const nextMode = other(rule.mode);

  const moveTargets = groups.filter((g) => g !== rule.group);
  const menuItems: ContextMenuItem[] = [
    {
      label: '移入分组',
      children: [
        ...moveTargets.map((g) => ({ label: g, onClick: () => onMoveToGroup(g) })),
        ...(moveTargets.length > 0 ? [{ type: 'divider' as const }] : []),
        { label: '新建分组', onClick: onNewGroupWith },
      ],
    },
    ...(rule.group ? [{ label: '移出分组', onClick: onRemoveFromGroup }] : []),
    { label: `改为${MODE_NAME[nextMode]}连发`, onClick: () => onSetMode(nextMode) },
    { type: 'divider' },
    { label: '删除', danger: true, onClick: onDelete },
  ];

  const cls = [
    'rule-row',
    isToggle ? 'is-toggle' : 'is-hold',
    !rule.enabled && 'disabled',
    isActive && 'active',
    isPaused && 'is-paused',
    isDragging && 'dragging',
    isDragTarget && 'drag-target',
  ]
    .filter(Boolean)
    .join(' ');

  // 长按不开高级时启动键跟随连发按键（重合态）；切换连发的启动键始终单独可设。
  const targetPolicy =
    isToggle || holdSplit || showAdvanced ? policies.target : policies.trigger_target;

  return (
    <div
      className={cls}
      title={isPaused ? '已暂停：同组长按插队中，松手后恢复' : undefined}
      data-tour={isLatest ? 'rule-latest' : undefined}
      draggable
      {...drag}
    >
      <div className="rule-head">
        <button
          type="button"
          className={`rule-mode-tag ${isToggle ? 'is-toggle' : 'is-hold'}`}
          data-tour="rule-mode"
          aria-label={`切换为${MODE_NAME[nextMode]}连发`}
          title={`${MODE_NAME[rule.mode]}连发（点击改为${MODE_NAME[nextMode]}连发）`}
          onClick={() => onSetMode(nextMode)}
        >
          {MODE_NAME[rule.mode]}
        </button>
        <span className="drag-handle" aria-hidden>
          <svg width="18" height="10" viewBox="0 0 18 10" fill="currentColor">
            <circle cx="3" cy="2" r="1.2" />
            <circle cx="9" cy="2" r="1.2" />
            <circle cx="15" cy="2" r="1.2" />
            <circle cx="3" cy="8" r="1.2" />
            <circle cx="9" cy="8" r="1.2" />
            <circle cx="15" cy="8" r="1.2" />
          </svg>
        </span>
        <button
          ref={menuRef}
          type="button"
          className="rule-menu-btn"
          data-tour="rule-menu"
          aria-label="更多操作"
          title="更多操作"
          onClick={() => setMenuOpen((v) => !v)}
        >
          <ChevronIcon size={12} />
        </button>
      </div>
      <div className="rule-body">
        <div className="rule-main">
          {showTrigger && (
            <>
              <div className="rule-field">
                <label>{isToggle ? '启动热键' : '长按键'}</label>
                <KeyCapture
                  onReject={onKeyReject}
                  policy={policies.trigger}
                  value={rule.trigger_key}
                  onChange={(vk) => vk && onPatch({ trigger_key: vk })}
                  conflict={conflict}
                />
              </div>
              <span className="rule-arrow">→</span>
            </>
          )}
          <div className="rule-field" data-tour="rule-key">
            <label>{multiple ? '输出队列' : '连发按键'}</label>
            {multiple ? (
              <button
                type="button"
                className="key-capture sequence-open"
                onClick={() => setSequenceOpen(true)}
              >
                {rule.steps.length} 步 · 编辑队列
              </button>
            ) : (
              <KeyCapture
                onReject={onKeyReject}
                policy={targetPolicy}
                value={rule.steps[0].key}
                onChange={(vk) => {
                  if (!vk) return;
                  const patch: Partial<CardRule> = { steps: [{ ...rule.steps[0], key: vk }] };
                  if (!showTrigger && !showAdvanced) patch.trigger_key = vk;
                  onPatch(patch);
                  onTargetCaptured();
                }}
                conflict={showTrigger || !showAdvanced ? conflict : null}
              />
            )}
          </div>
          {!multiple && (
            <div className="rule-field rule-interval" data-tour="rule-interval">
              <label>间隔</label>
              <IntervalInput
                value={rule.steps[0].interval_ms}
                min={intervalMin}
                max={intervalMax}
                onChange={(v) => onPatch({ steps: [{ ...rule.steps[0], interval_ms: v }] })}
              />
            </div>
          )}
        </div>
        <input
          type="checkbox"
          className="enable-checkbox"
          data-tour="rule-enable"
          checked={rule.enabled}
          onChange={(e) => onPatch({ enabled: e.target.checked })}
          aria-label="启用"
        />
      </div>
      {!multiple && !sequenceOpen && (
        <button
          type="button"
          className="expand-btn"
          data-tour="sequence-add"
          onClick={() => setSequenceOpen(true)}
        >
          ＋ 添加步骤／录制队列
        </button>
      )}
      {sequenceOpen && (
        <SequenceEditor
          steps={rule.steps}
          policy={policies.target}
          min={intervalMin}
          max={intervalMax}
          onReject={onKeyReject}
          onClose={() => setSequenceOpen(false)}
          onSave={onSaveSequence}
        />
      )}
      {progress && (multiple || progress.error || progress.max_lateness_ms > 0) && (
        <p className="sequence-progress" role="status">
          {progress.error ??
            `${isPaused ? '暂停' : '执行'}第 ${progress.step_index + 1}/${progress.step_count} 步 · 已完成 ${progress.completed_cycles} 轮${progress.max_lateness_ms > 0 ? ` · 最近延后 ${progress.lateness_ms}ms，最大 ${progress.max_lateness_ms}ms` : ''}`}
        </p>
      )}
      {showAdvanced && (
        <div className="rule-advanced">
          {isToggle ? (
            <>
              <div className="rule-field">
                <label>停止热键</label>
                <KeyCapture
                  onReject={onKeyReject}
                  policy={policies.trigger}
                  value={rule.stop_key ?? rule.trigger_key}
                  onChange={(vk) => vk && onPatch({ stop_key: vk })}
                />
              </div>
              <span className="adv-hint">默认与启动热键相同</span>
            </>
          ) : holdSplit ? (
            <span className="adv-hint">长按键已在上面单独设置；改成与连发按键相同即回到单键</span>
          ) : (
            <>
              <div className="rule-field">
                <label>长按键</label>
                <KeyCapture
                  onReject={onKeyReject}
                  policy={policies.trigger}
                  value={rule.trigger_key}
                  onChange={(vk) => vk && onPatch({ trigger_key: vk })}
                  conflict={conflict}
                />
              </div>
              <span className="adv-hint">默认与连发按键相同</span>
            </>
          )}
        </div>
      )}
      <button
        className={`expand-btn${showAdvanced ? ' open' : ''}`}
        data-tour="rule-advanced"
        onClick={onToggleAdvanced}
        aria-label="高级设置"
      >
        <ChevronIcon size={10} className="chevron" />
        <span className="expand-label">{showAdvanced ? '收起高级设置' : '高级设置'}</span>
      </button>
      <ContextMenu
        open={menuOpen}
        onClose={() => setMenuOpen(false)}
        target={menuRef}
        items={menuItems}
        location="bottom-left"
      />
    </div>
  );
}
