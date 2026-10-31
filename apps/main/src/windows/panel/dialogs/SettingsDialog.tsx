import { type ReactNode, useCallback, useState } from 'react';
import Button from '../components/Button';
import { CardList, CardListButton } from '../components/CardList';
import type { CloseBehavior } from '../components/CloseBehaviorForm';
import KeyCapture, {
  type CaptureReject,
  keyboardKey,
  keyEq,
  type KeyId,
  MODIFIER_VK,
  type SlotPolicy,
} from '../components/KeyCapture';
import { VolumeIcon } from '../components/icons';
import Tabs from '../components/Tabs';
import { useToast } from '../components/Toast';
import type { ConflictSeverity } from '../conflicts';
import { clampSignature } from '../license';
import DialogShell from './DialogShell';
import ProfileCardList, { type SettingsProfileEntry } from './ProfileCardList';
import { SECT_PRESETS, type ThemeMode, type ThemeSettings } from '../theme';
import './SettingsDialog.css';

export type SettingsTab = 'general' | 'hotkeys' | 'sound' | 'profiles';
type SettingsInputMode = 'sendinput' | 'interception' | 'ddsimple' | 'hidstick';
type DriverStatus = 'installed' | 'pending_reboot' | 'not_installed';

/** 四个播报时机，也是 `${slot}Text` / `${slot}Source` 等字段的前缀。 */
export type SoundSlot = 'start' | 'end' | 'toggleStart' | 'toggleEnd';

/** 每个时机各自选择用 TTS 朗读文本，还是播放用户自选的音频文件。 */
export type SoundSource = 'tts' | 'audio';

export interface SoundSettings {
  enabled: boolean;
  // 每个播报时机独立开关，默认开；总开关 enabled 关闭时整体静音
  startEnabled: boolean;
  endEnabled: boolean;
  toggleStartEnabled: boolean;
  toggleEndEnabled: boolean;
  volume: number;
  rate: number;
  pitch: number;
  startText: string;
  endText: string;
  toggleStartText: string;
  toggleEndText: string;
  // 音源与音频文件名逐时机独立：常见诉求是全局开关配音效、Toggle 仍朗读键名
  startSource: SoundSource;
  endSource: SoundSource;
  toggleStartSource: SoundSource;
  toggleEndSource: SoundSource;
  // 存 {app_data_dir}/sounds/ 下的文件名而非源路径，空串表示尚未选择
  startAudio: string;
  endAudio: string;
  toggleStartAudio: string;
  toggleEndAudio: string;
  voiceName: string;
  globalOnly: boolean; // reserved, not yet wired
}

interface Props {
  // 受控：引导教程要在弹窗已打开时切页签，内部 state 做不到（组件不重挂载）
  tab: SettingsTab;
  onTabChange: (tab: SettingsTab) => void;
  appVersion: string;
  inputMode: SettingsInputMode;
  layout: 'vertical' | 'horizontal';
  /** 当前布局的窗口缩放比例，1 为设计尺寸 */
  windowScale: number;
  onResetWindowScale: () => void;
  switchingMode: boolean;
  globalEnabled: boolean;
  togglingGlobal: boolean;
  elevated: boolean;
  closeBehavior: CloseBehavior | null;
  interceptionInstalled: DriverStatus;
  ddHidInstalled: DriverStatus;
  hidStickPresent: boolean;
  autostartEnabled: boolean;
  togglingAutostart: boolean;
  autoEnableOnStart: boolean;
  runAsAdmin: boolean;
  togglingRunAsAdmin: boolean;
  autoUpdate: boolean;
  sound: SoundSettings;
  availableVoices: string[];
  profiles: SettingsProfileEntry[];
  profileName: string;
  profileCount: number;
  isDefaultProfile: boolean;
  /** 持有 `flair.signature` 权益；没有时界面完全不露署名入口。 */
  signatureEnabled: boolean;
  signature: string;
  onSignatureChange: (signature: string) => void;
  onClose: () => void;
  onSelectInputMode: (mode: SettingsInputMode) => void;
  onToggleGlobal: () => void;
  onSetCloseBehavior: (choice: CloseBehavior | null) => void;
  hotkeys: { global_toggle: KeyId | null; global_stop: KeyId | null; panel_toggle: KeyId | null };
  /** 热键槽的允许集，来自后端 `get_key_policy`。 */
  hotkeyPolicy: SlotPolicy;
  hotkeyConflicts: {
    global_toggle: ConflictSeverity | null;
    global_stop: ConflictSeverity | null;
    panel_toggle: ConflictSeverity | null;
  };
  onHotkeyChange: (patch: {
    global_toggle?: KeyId | null;
    global_stop?: KeyId | null;
    panel_toggle?: KeyId | null;
  }) => void;
  onToggleAutostart: () => void;
  onToggleAutoEnableOnStart: (next: boolean) => void;
  onToggleRunAsAdmin: () => void;
  onToggleAutoUpdate: (next: boolean) => void;
  onSoundChange: (patch: Partial<SoundSettings>) => void;
  onPreviewSound: (slot: SoundSlot) => void;
  onPickSoundAudio: (slot: SoundSlot) => void;
  theme: ThemeSettings;
  onThemeChange: (patch: Partial<ThemeSettings>) => void;
  onCreateProfile: () => void;
  onImportProfile: () => void;
  onImportExternal: () => void;
  onSwitchProfile: (path: string) => void;
  onRenameProfile: (name: string) => void;
  onDeleteProfile: (name: string) => void;
  onExportProfile: (name: string) => void;
}

const TABS: { id: SettingsTab; label: string }[] = [
  { id: 'general', label: '通用' },
  { id: 'hotkeys', label: '热键' },
  { id: 'sound', label: '声音' },
  { id: 'profiles', label: '配置文件' },
];

const DEFAULT_PROFILE_NAME = 'defaults';

const THEME_MODE_OPTIONS: { value: ThemeMode; label: string }[] = [
  { value: 'light', label: '亮' },
  { value: 'dark', label: '暗' },
  { value: 'system', label: '跟随系统' },
];

const SOUND_SOURCE_OPTIONS: { value: SoundSource; label: string }[] = [
  { value: 'tts', label: '朗读' },
  { value: 'audio', label: '音频' },
];

const INPUT_MODE_LABELS: Record<SettingsInputMode, string> = {
  sendinput: '通用模式',
  interception: '游戏模式',
  ddsimple: 'DD驱动',
  hidstick: '外设模式',
};

const INPUT_MODE_HINTS: Record<SettingsInputMode, string> = {
  sendinput: 'SendInput',
  interception: 'Interception',
  ddsimple: 'DD驱动',
  hidstick: '外设驱动',
};

const CLOSE_BEHAVIOR_OPTIONS: {
  value: CloseBehavior | null;
  label: string;
  detail: string;
}[] = [
  { value: 'minimize', label: '切换到悬浮窗', detail: '收起为悬浮窗' },
  { value: 'exit', label: '直接退出', detail: '关闭应用进程' },
  { value: null, label: '关闭时询问', detail: '每次确认' },
];

function SettingsSection({
  title,
  children,
  dataTour,
}: {
  title: string;
  children: ReactNode;
  dataTour?: string;
}) {
  return (
    <section className="settings-section" data-tour={dataTour}>
      <h3 className="settings-section-title">{title}</h3>
      {children}
    </section>
  );
}

function SignatureRow({
  value,
  onCommit,
}: {
  value: string;
  onCommit: (signature: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  // 失焦或回车才写盘：每敲一个字都写会让配置文件跟着输入法候选一起抖
  const commit = () => {
    if (draft.trim() !== value) onCommit(draft);
  };
  return (
    <div className="settings-row settings-row--stack">
      <div className="settings-row-main">
        <span className="settings-row-title">面板背景署名</span>
        <span className="settings-row-desc">跟着当前配置走，留空显示默认署名</span>
      </div>
      <input
        type="text"
        className="settings-text-input"
        value={draft}
        placeholder="花间游悟 · 斗转星移"
        aria-label="面板背景署名"
        onChange={(e) => setDraft(clampSignature(e.target.value))}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') e.currentTarget.blur();
        }}
      />
    </div>
  );
}

function driverStatusLabel(status: DriverStatus): string {
  if (status === 'installed') return '已安装';
  if (status === 'pending_reboot') return '待重启';
  return '未安装';
}

function modeDetail(mode: SettingsInputMode, props: Props): string {
  if (mode === 'interception') {
    if (props.interceptionInstalled !== 'installed') {
      return driverStatusLabel(props.interceptionInstalled);
    }
    return props.elevated ? '管理员已就绪' : '需要管理员';
  }
  if (mode === 'ddsimple') {
    return props.elevated ? '管理员已就绪' : '需要管理员';
  }
  if (mode === 'hidstick') {
    return props.hidStickPresent ? '已插入' : '未插入';
  }
  return '无需驱动';
}

/// 模式角标：游戏模式主推（推荐）、DD驱动降级（备用）。通用模式无角标。
function modeTag(mode: SettingsInputMode): { text: string; kind: 'recommend' | 'backup' } | null {
  if (mode === 'interception') return { text: '推荐', kind: 'recommend' };
  if (mode === 'ddsimple') return { text: '备用', kind: 'backup' };
  return null;
}

/// 可选输入模式：游戏模式置顶主推，通用模式次之，DD驱动仅作备用；外设模式要有外设驱动，放最后。
const SELECTABLE_INPUT_MODES: SettingsInputMode[] = [
  'interception',
  'sendinput',
  'ddsimple',
  'hidstick',
];

function SliderRow({
  label,
  value,
  min,
  max,
  onChange,
  formatValue,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onChange: (v: number) => void;
  formatValue?: (v: number) => string;
}) {
  return (
    <div className="settings-row">
      <div className="settings-row-main">
        <span className="settings-row-title">{label}</span>
      </div>
      <div className="settings-slider-group">
        <input
          type="range"
          className="settings-slider"
          min={min}
          max={max}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
        />
        <span className="settings-slider-value">{formatValue ? formatValue(value) : value}</span>
      </div>
    </div>
  );
}

// 单条播报行，两行结构：标题 + 音源分段（朗读 / 音频）+ 行尾开关，下面一整行随音源
// 切换为文本框或音频选择按钮（右端内嵌试听）。
// 编辑与试听仅受总开关 masterEnabled 限制；行尾开关仅控制实际播报，不锁定编辑。
function SoundStatementRow({
  title,
  value,
  source,
  audioName,
  enabled,
  masterEnabled,
  onTextChange,
  onSourceChange,
  onPickAudio,
  onToggle,
  onPreview,
}: {
  title: string;
  value: string;
  source: SoundSource;
  audioName: string;
  enabled: boolean;
  masterEnabled: boolean;
  onTextChange: (v: string) => void;
  onSourceChange: (s: SoundSource) => void;
  onPickAudio: () => void;
  onToggle: (on: boolean) => void;
  onPreview: () => void;
}) {
  const isAudio = source === 'audio';
  return (
    <div className="settings-row settings-row--stack settings-sound-row">
      <div className="settings-sound-head">
        <span className="settings-row-title">{title}</span>
        <div className="settings-sound-head-actions">
          <div className="settings-source-seg" role="group" aria-label={`${title}音源`}>
            {SOUND_SOURCE_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                type="button"
                className={`settings-source-seg-btn${source === opt.value ? ' settings-source-seg-btn--active' : ''}`}
                disabled={!masterEnabled}
                aria-pressed={source === opt.value}
                onClick={() => onSourceChange(opt.value)}
              >
                {opt.label}
              </button>
            ))}
          </div>
          <input
            type="checkbox"
            className="enable-checkbox"
            checked={enabled}
            disabled={!masterEnabled}
            onChange={(e) => onToggle(e.target.checked)}
            aria-label={`${title}提示开关`}
          />
        </div>
      </div>
      <div className="settings-input-wrap">
        {isAudio ? (
          <button
            type="button"
            className={`settings-audio-pick${audioName ? '' : ' settings-audio-pick--empty'}`}
            disabled={!masterEnabled}
            onClick={onPickAudio}
            title={audioName || '选择音频文件'}
          >
            {audioName || '选择音频文件…'}
          </button>
        ) : (
          <input
            type="text"
            className="settings-text-input"
            value={value}
            maxLength={30}
            disabled={!masterEnabled}
            onChange={(e) => onTextChange(e.target.value)}
          />
        )}
        <button
          type="button"
          className="settings-input-icon-btn"
          disabled={!masterEnabled || (isAudio && !audioName)}
          onClick={onPreview}
          aria-label="试听"
          title="试听"
        >
          <VolumeIcon />
        </button>
      </div>
    </div>
  );
}

export default function SettingsDialog(props: Props) {
  const tab = props.tab;
  const [hotkeyDupNote, setHotkeyDupNote] = useState<string | null>(null);
  const { sound } = props;
  const toast = useToast();

  // KeyCapture 查表落空时会静默丢弃这次按键，用户只看到按钮闪一下，无从判断是没按上还是不支持。
  // 回调稳定引用：KeyCapture 的监听 effect 依赖 onReject，内联函数会导致每次渲染重挂监听。
  const notifyHotkeyReject = useCallback(
    (info: CaptureReject) => {
      if (info.reason === 'mouse' || info.reason === 'mouse-unsupported') {
        toast.warning('全局热键只能绑定键盘按键，鼠标按键与滚轮请用在连发规则上');
        return;
      }
      // 热键槽的 keyboard 恒为 true，slot-disabled 到不了这里，兜底当作不支持处理
      if (info.reason === 'slot-disabled') {
        toast.warning('这个按键当前不可绑定，请换一个');
        return;
      }
      toast.warning(`不支持绑定这个按键（${info.code}），请换一个`);
    },
    [toast],
  );

  // AltGr 布局（德语 / 法语 / 波兰语等）把右 Alt 当 AltGr 用，键盘驱动会在它前面补发一个左 Ctrl，
  // 低级钩子看到的是「左 Ctrl 按下 + 右 Alt 按下」两个事件。所以这两个键同时绑热键会互相牵连，
  // 只绑右 Alt 则要提醒别再占用左 Ctrl。中文 / 英文常用布局的右 Alt 是普通 Alt，不受影响。
  const boundHotkeys = [
    props.hotkeys.global_toggle,
    props.hotkeys.global_stop,
    props.hotkeys.panel_toggle,
  ];
  const rightAltBound = boundHotkeys.some((k) => keyEq(k, keyboardKey(MODIFIER_VK.AltRight)));
  const leftCtrlBound = boundHotkeys.some((k) => keyEq(k, keyboardKey(MODIFIER_VK.ControlLeft)));

  // 全局热键只能绑定键盘实体键（KeyCapture keyboardOnly），且三者互不重复——重复会让该键被
  // 某个热键抢先处理、其余功能失效，行为不可预期。绑定前若与另一全局热键相同则拒绝并提示。
  const setGlobalHotkey = (
    field: 'global_toggle' | 'global_stop' | 'panel_toggle',
    key: KeyId | null,
  ) => {
    if (key) {
      const others = (['global_toggle', 'global_stop', 'panel_toggle'] as const).filter(
        (f) => f !== field,
      );
      if (others.some((f) => keyEq(props.hotkeys[f], key))) {
        setHotkeyDupNote('该按键已被其它全局热键占用，请换一个');
        return;
      }
    }
    setHotkeyDupNote(null);
    if (field === 'global_toggle') {
      props.onHotkeyChange({
        global_toggle: key,
        global_stop: key === null ? null : props.hotkeys.global_stop,
      });
    } else if (field === 'global_stop') {
      props.onHotkeyChange({ global_stop: key });
    } else {
      props.onHotkeyChange({ panel_toggle: key });
    }
  };

  const tabsNode = (
    <div data-tour="settings-tabs">
      <Tabs tabs={TABS} active={tab} onChange={props.onTabChange} variant="pill" grow />
    </div>
  );

  return (
    <DialogShell
      className="settings-card"
      title="设置"
      labelId="settings-title"
      subheader={tabsNode}
      footer={<Button onClick={props.onClose}>关闭</Button>}
    >
      <div className="settings-body">
        {tab === 'general' && (
          <>
            <SettingsSection title="外观" dataTour="settings-general-theme">
              <div className="settings-row">
                <div className="settings-row-main">
                  <span className="settings-row-title">明暗模式</span>
                  <span className="settings-row-desc">跟随系统或手动锁定亮/暗</span>
                </div>
                <div className="theme-mode-seg">
                  {THEME_MODE_OPTIONS.map((opt) => (
                    <button
                      key={opt.value}
                      type="button"
                      className={`theme-mode-seg-btn${
                        props.theme.mode === opt.value ? ' theme-mode-seg-btn--active' : ''
                      }`}
                      onClick={() => props.onThemeChange({ mode: opt.value })}
                    >
                      {opt.label}
                    </button>
                  ))}
                </div>
              </div>
              <div className="settings-row settings-row--stack">
                <div className="settings-row-main">
                  <span className="settings-row-title">主题色</span>
                  <span className="settings-row-desc">移入查看配色名称</span>
                </div>
                <div className="theme-swatches">
                  {SECT_PRESETS.map((p) => (
                    <button
                      key={p.id}
                      type="button"
                      className={`theme-swatch${
                        props.theme.color.toLowerCase() === p.color.toLowerCase()
                          ? ' theme-swatch--active'
                          : ''
                      }`}
                      style={{ background: p.color }}
                      title={p.name}
                      aria-label={p.name}
                      onClick={() => props.onThemeChange({ color: p.color })}
                    />
                  ))}
                </div>
              </div>
              <div className="settings-row">
                <div className="settings-row-main">
                  <span className="settings-row-title">
                    界面缩放 {Math.round(props.windowScale * 100)}%
                  </span>
                  <span className="settings-row-desc">
                    拖动面板边缘等比放大，竖版、横版分别记住
                  </span>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  tone="neutral"
                  disabled={props.windowScale === 1}
                  onClick={props.onResetWindowScale}
                >
                  复位
                </Button>
              </div>
            </SettingsSection>

            <SettingsSection title="运行" dataTour="settings-general-startup">
              <div className="settings-row">
                <div className="settings-row-main">
                  <span className="settings-row-title">全局开关</span>
                  <span className="settings-row-desc">
                    {props.globalEnabled ? '当前启用' : '当前停用'}
                  </span>
                </div>
                <Button
                  size="sm"
                  tone={props.globalEnabled ? 'primary' : 'neutral'}
                  loading={props.togglingGlobal}
                  onClick={props.onToggleGlobal}
                >
                  {props.globalEnabled ? '已启用' : '已禁用'}
                </Button>
              </div>
              <div className="settings-row">
                <div className="settings-row-main">
                  <span className="settings-row-title">启动后自动开全局</span>
                  <span className="settings-row-desc">
                    {props.autoEnableOnStart
                      ? '打开应用即开始连发，与开机自启互斥'
                      : '打开应用后仍需手动开全局'}
                  </span>
                </div>
                <input
                  type="checkbox"
                  className="enable-checkbox"
                  checked={props.autoEnableOnStart}
                  onChange={(e) => props.onToggleAutoEnableOnStart(e.target.checked)}
                  aria-label="启动后自动开全局"
                />
              </div>
              <div className="settings-row">
                <div className="settings-row-main">
                  <span className="settings-row-title">开机自启</span>
                  <span className="settings-row-desc">
                    {props.autostartEnabled ? '登录后自动启动' : '不自动启动'}
                  </span>
                </div>
                <Button
                  size="sm"
                  tone={props.autostartEnabled ? 'primary' : 'neutral'}
                  loading={props.togglingAutostart}
                  onClick={props.onToggleAutostart}
                >
                  {props.autostartEnabled ? '已启用' : '已禁用'}
                </Button>
              </div>
              <div className="settings-row">
                <div className="settings-row-main">
                  <span className="settings-row-title">以管理员模式启动</span>
                  <span className="settings-row-desc">
                    {props.runAsAdmin
                      ? '下次启动自动请求管理员权限，游戏模式不用再提权重启'
                      : '普通权限启动，游戏模式需要时再提权重启'}
                  </span>
                </div>
                <Button
                  size="sm"
                  tone={props.runAsAdmin ? 'primary' : 'neutral'}
                  loading={props.togglingRunAsAdmin}
                  onClick={props.onToggleRunAsAdmin}
                >
                  {props.runAsAdmin ? '已启用' : '已禁用'}
                </Button>
              </div>
              <div className="settings-row">
                <div className="settings-row-main">
                  <span className="settings-row-title">自动更新</span>
                  <span className="settings-row-desc">
                    {props.autoUpdate
                      ? '后台静默下载新版本，下载完成后提示重启安装'
                      : '仅在标题栏提示有新版本，由你决定何时下载'}
                  </span>
                </div>
                <input
                  type="checkbox"
                  className="enable-checkbox"
                  checked={props.autoUpdate}
                  onChange={(e) => props.onToggleAutoUpdate(e.target.checked)}
                  aria-label="自动更新"
                />
              </div>
            </SettingsSection>

            <SettingsSection title="输入模式">
              <CardList>
                {SELECTABLE_INPUT_MODES.map((mode) => {
                  const tag = modeTag(mode);
                  // 外设模式仍限制同键切换连发，横版下禁用。
                  const layoutBlocked = props.layout === 'horizontal' && mode === 'hidstick';
                  return (
                    <CardListButton
                      key={mode}
                      active={props.inputMode === mode}
                      className="settings-mode"
                      disabled={props.switchingMode || layoutBlocked}
                      onClick={() => props.onSelectInputMode(mode)}
                    >
                      <span className="settings-mode-name">
                        {INPUT_MODE_LABELS[mode]}
                        {tag && (
                          <span className={`settings-mode-tag settings-mode-tag-${tag.kind}`}>
                            {tag.text}
                          </span>
                        )}
                      </span>
                      <span className="settings-mode-meta">
                        {layoutBlocked
                          ? '横版键鼠图下不可用，请先切回竖版'
                          : `${INPUT_MODE_HINTS[mode]} · ${modeDetail(mode, props)}`}
                      </span>
                    </CardListButton>
                  );
                })}
              </CardList>
              <p className="settings-note settings-note-warn">
                ⚠️ DD 驱动可能无法正确停止连发、甚至自行停止连发，仅在「游戏模式」不可用时作为备用。
                同键启停异常时可关闭全局开关或正常退出应用，停止连发并释放按键。优先使用游戏模式。
              </p>
              <p className="settings-note">驱动安装与卸载操作请前往「诊断修复」。</p>
            </SettingsSection>

            <SettingsSection title="关闭行为" dataTour="settings-general-close">
              <CardList columns="three" role="radiogroup" aria-label="关闭行为">
                {CLOSE_BEHAVIOR_OPTIONS.map((item) => (
                  <CardListButton
                    key={item.value ?? 'ask'}
                    role="radio"
                    aria-checked={props.closeBehavior === item.value}
                    active={props.closeBehavior === item.value}
                    className="settings-choice"
                    onClick={() => props.onSetCloseBehavior(item.value)}
                  >
                    <span>{item.label}</span>
                    <small>{item.detail}</small>
                  </CardListButton>
                ))}
              </CardList>
            </SettingsSection>
          </>
        )}

        {tab === 'hotkeys' && (
          <SettingsSection title="热键" dataTour="settings-hotkeys">
            <div className="settings-hotkey-list">
              <div className="settings-hotkey-row">
                <span className="settings-hotkey-label">全局开关</span>
                <div className="settings-hotkey-keys">
                  <KeyCapture
                    value={props.hotkeys.global_toggle}
                    nullable
                    policy={props.hotkeyPolicy}
                    onReject={notifyHotkeyReject}
                    placeholder="未设置"
                    conflict={props.hotkeyConflicts.global_toggle}
                    onChange={(k) => setGlobalHotkey('global_toggle', k)}
                  />
                  {props.hotkeys.global_toggle && (
                    <>
                      <span className="settings-hotkey-sep">停止</span>
                      <KeyCapture
                        value={props.hotkeys.global_stop}
                        nullable
                        policy={props.hotkeyPolicy}
                        onReject={notifyHotkeyReject}
                        placeholder="同开启键"
                        conflict={props.hotkeyConflicts.global_stop}
                        onChange={(k) => setGlobalHotkey('global_stop', k)}
                      />
                    </>
                  )}
                </div>
              </div>
              <div className="settings-hotkey-row">
                <span className="settings-hotkey-label">面板显隐</span>
                <div className="settings-hotkey-keys">
                  <KeyCapture
                    value={props.hotkeys.panel_toggle}
                    nullable
                    policy={props.hotkeyPolicy}
                    onReject={notifyHotkeyReject}
                    placeholder="未设置"
                    conflict={props.hotkeyConflicts.panel_toggle}
                    onChange={(k) => setGlobalHotkey('panel_toggle', k)}
                  />
                </div>
              </div>
            </div>
            {hotkeyDupNote && <p className="settings-note settings-note-warn">{hotkeyDupNote}</p>}
            {rightAltBound &&
              (leftCtrlBound ? (
                <p className="settings-note settings-note-warn">
                  右 Alt 和左 Ctrl 同时绑了热键。如果你用的是德语 / 法语 / 波兰语等带 AltGr
                  的键盘布局，右 Alt 就是 AltGr，按下时系统会先补发一个左
                  Ctrl，于是两个热键会一起触发。 用这类布局请换掉其中一个；中文 /
                  英文常用布局不受影响。
                </p>
              ) : (
                <p className="settings-note">
                  已绑定右 Alt。带 AltGr 的键盘布局（德语 / 法语 / 波兰语等）下右 Alt 就是
                  AltGr，按下时系统会先补发一个左 Ctrl，所以别再把左 Ctrl 绑给另一个热键。中文 /
                  英文常用布局的右 Alt 是普通 Alt，不受影响。
                </p>
              ))}
            <p className="settings-note">
              点按键框开始捕获，随后按下要绑定的键；已绑定的键框上点右键可清除。
            </p>
            <p className="settings-note">
              全局热键仅支持键盘按键（含左右 Shift / Ctrl / Alt /
              Win），三个热键不能重复；热键随当前配置文件保存。
            </p>
          </SettingsSection>
        )}

        {tab === 'sound' && (
          <>
            <SettingsSection title="声音反馈" dataTour="settings-sound">
              <div className="settings-row">
                <div className="settings-row-main">
                  <span className="settings-row-title">声音反馈</span>
                  <span className="settings-row-desc">
                    {sound.enabled ? '连发状态变化时朗读语句或播放音频' : '已关闭'}
                  </span>
                </div>
                <input
                  type="checkbox"
                  className="enable-checkbox"
                  checked={sound.enabled}
                  onChange={(e) => props.onSoundChange({ enabled: e.target.checked })}
                  aria-label="声音反馈"
                />
              </div>
            </SettingsSection>

            <SettingsSection title="提示">
              <div
                className={`settings-sound-body${sound.enabled ? '' : ' settings-sound-body--disabled'}`}
              >
                <SoundStatementRow
                  title="全局启用"
                  value={sound.startText}
                  source={sound.startSource}
                  audioName={sound.startAudio}
                  enabled={sound.startEnabled}
                  masterEnabled={sound.enabled}
                  onTextChange={(v) => props.onSoundChange({ startText: v })}
                  onSourceChange={(s) => props.onSoundChange({ startSource: s })}
                  onPickAudio={() => props.onPickSoundAudio('start')}
                  onToggle={(on) => props.onSoundChange({ startEnabled: on })}
                  onPreview={() => props.onPreviewSound('start')}
                />
                <SoundStatementRow
                  title="全局停用"
                  value={sound.endText}
                  source={sound.endSource}
                  audioName={sound.endAudio}
                  enabled={sound.endEnabled}
                  masterEnabled={sound.enabled}
                  onTextChange={(v) => props.onSoundChange({ endText: v })}
                  onSourceChange={(s) => props.onSoundChange({ endSource: s })}
                  onPickAudio={() => props.onPickSoundAudio('end')}
                  onToggle={(on) => props.onSoundChange({ endEnabled: on })}
                  onPreview={() => props.onPreviewSound('end')}
                />
                <SoundStatementRow
                  title="切换连发开始"
                  value={sound.toggleStartText}
                  source={sound.toggleStartSource}
                  audioName={sound.toggleStartAudio}
                  enabled={sound.toggleStartEnabled}
                  masterEnabled={sound.enabled}
                  onTextChange={(v) => props.onSoundChange({ toggleStartText: v })}
                  onSourceChange={(s) => props.onSoundChange({ toggleStartSource: s })}
                  onPickAudio={() => props.onPickSoundAudio('toggleStart')}
                  onToggle={(on) => props.onSoundChange({ toggleStartEnabled: on })}
                  onPreview={() => props.onPreviewSound('toggleStart')}
                />
                <SoundStatementRow
                  title="切换连发结束"
                  value={sound.toggleEndText}
                  source={sound.toggleEndSource}
                  audioName={sound.toggleEndAudio}
                  enabled={sound.toggleEndEnabled}
                  masterEnabled={sound.enabled}
                  onTextChange={(v) => props.onSoundChange({ toggleEndText: v })}
                  onSourceChange={(s) => props.onSoundChange({ toggleEndSource: s })}
                  onPickAudio={() => props.onPickSoundAudio('toggleEnd')}
                  onToggle={(on) => props.onSoundChange({ toggleEndEnabled: on })}
                  onPreview={() => props.onPreviewSound('toggleEnd')}
                />
              </div>
              <p className="settings-note">
                Toggle 两条语句里的 {'${key}'} 会替换为规则的目标键名。
              </p>
              <p className="settings-note">
                选「音频」后文件会复制进应用数据目录，之后移动或删除源文件不影响播放。支持 mp3 / wav
                / ogg / m4a / flac，单个不超过 5 MB。
              </p>
            </SettingsSection>

            <SettingsSection title="合成参数">
              <div
                className={`settings-sound-body${sound.enabled ? '' : ' settings-sound-body--disabled'}`}
              >
                <div className="settings-row">
                  <div className="settings-row-main">
                    <span className="settings-row-title">语音</span>
                  </div>
                  <select
                    className="settings-select"
                    value={sound.voiceName}
                    disabled={!sound.enabled}
                    onChange={(e) => props.onSoundChange({ voiceName: e.target.value })}
                  >
                    {props.availableVoices.length === 0 ? (
                      <option value="">系统无可用语音</option>
                    ) : (
                      <>
                        <option value="">系统默认</option>
                        {props.availableVoices.map((v) => (
                          <option key={v} value={v}>
                            {v}
                          </option>
                        ))}
                      </>
                    )}
                  </select>
                </div>

                <SliderRow
                  label="语速"
                  value={sound.rate}
                  min={-10}
                  max={10}
                  onChange={(v) => props.onSoundChange({ rate: v })}
                  formatValue={(v) => (v === 0 ? '0' : v > 0 ? `+${v}` : `${v}`)}
                />
                <SliderRow
                  label="音调"
                  value={sound.pitch}
                  min={-10}
                  max={10}
                  onChange={(v) => props.onSoundChange({ pitch: v })}
                  formatValue={(v) => (v === 0 ? '0' : v > 0 ? `+${v}` : `${v}`)}
                />
                <SliderRow
                  label="音量"
                  value={sound.volume}
                  min={0}
                  max={100}
                  onChange={(v) => props.onSoundChange({ volume: v })}
                  formatValue={(v) => `${v}%`}
                />
              </div>
              <p className="settings-note">
                语音、语速、音调只作用于朗读；音量对朗读与音频都生效。
              </p>
            </SettingsSection>
          </>
        )}

        {tab === 'profiles' && (
          <>
            <SettingsSection title="我的配置">
              <div className="settings-profile-toolbar">
                <span>{props.profileCount} 个配置</span>
                <div className="settings-profile-toolbar-actions">
                  <Button
                    variant="outline"
                    tone="primary"
                    size="sm"
                    onClick={props.onCreateProfile}
                  >
                    新建配置
                  </Button>
                  <Button
                    variant="outline"
                    tone="neutral"
                    size="sm"
                    onClick={props.onImportProfile}
                  >
                    导入配置
                  </Button>
                  <Button
                    variant="outline"
                    tone="neutral"
                    size="sm"
                    onClick={props.onImportExternal}
                  >
                    外部导入
                  </Button>
                </div>
              </div>

              <ProfileCardList
                profiles={props.profiles}
                activeProfileName={props.profileName}
                defaultProfileName={DEFAULT_PROFILE_NAME}
                onSwitchProfile={props.onSwitchProfile}
                onRenameProfile={props.onRenameProfile}
                onDeleteProfile={props.onDeleteProfile}
                onExportProfile={props.onExportProfile}
              />

              {props.profiles.length === 0 && (
                <p className="settings-note">当前没有配置文件，可先新建或导入配置。</p>
              )}

              {props.isDefaultProfile && (
                <p className="settings-note">默认配置会在修改规则或热键时自动另存为新配置。</p>
              )}
            </SettingsSection>

            {props.signatureEnabled && (
              <SettingsSection title="水印署名">
                {/* 切配置或保存后署名变了就重挂载，丢掉旧草稿 */}
                <SignatureRow
                  key={`${props.profileName}\u0000${props.signature}`}
                  value={props.signature}
                  onCommit={props.onSignatureChange}
                />
              </SettingsSection>
            )}
          </>
        )}
      </div>
    </DialogShell>
  );
}
