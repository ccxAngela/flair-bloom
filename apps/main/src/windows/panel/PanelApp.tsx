import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open as openFileDialog, save as saveFileDialog } from '@tauri-apps/plugin-dialog';
import { LazyStore } from '@tauri-apps/plugin-store';
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from 'react';
import iconUrl from '../../assets/icon-32.png';
import bgUrl from '../../assets/icon.png';
import { APP_NAME, AUTHOR_EMAIL, WINDOWS_RESTART_HINT } from '../../constants';
import HorizontalLayout from './HorizontalLayout';
import RuleCard, { type CardDragHandlers } from './RuleCard';
import RuleGroup from './RuleGroup';
import Button from './components/Button';
import CloseBehaviorForm, { type CloseBehavior } from './components/CloseBehaviorForm';
import { useConfirm, useConfirmOpen } from './components/ConfirmDialog';
import ContextMenu, { type ContextMenuItem } from './components/ContextMenu';
import {
  ChevronIcon,
  CloseIcon,
  EditIcon,
  MenuIcon,
  MinimizeIcon,
  UpdateIcon,
} from './components/icons';
import Kbd from './components/Kbd';

import {
  type CaptureReject,
  type KeyPolicies,
  RESTRICTIVE_POLICY,
  keyboardKey,
  keyEq,
  keyLabel,
  mouseKey,
  type KeyId,
} from './components/KeyCapture';
import Overlay from './components/Overlay';
import ProfileNameForm from './components/ProfileNameForm';
import Tabs from './components/Tabs';
import { useToast } from './components/Toast';
import { useImportWelcome } from './useImportWelcome';
import UpdateProgressBar, { type UpdateDownloadProgress } from './components/UpdateProgressBar';
import { detectConflicts, severityForKey, severityForRule } from './conflicts';
import TourRunner from './tour/TourRunner';
import { findTour, TOURS } from './tour/tours';
import {
  isSampleRule,
  lastRule,
  SAMPLE_GROUP,
  SAMPLE_IDS,
  sampleRules,
} from './tour/tours/helpers';
import { keyToken } from './keyToken';
import type { TourDef, TourExitResult, TourHost, TourSnapshot } from './tour/types';
import { useTourProgress } from './tour/useTourProgress';
import { useKeyRelay } from './useKeyRelay';
import { useRuleStates } from './useRuleStates';
import { createProfilePersistence, type ProfileWriteTicket } from './profilePersistence';
import { createToggleFeedbackHandler, type ToggleFeedback } from './toggleFeedback';
import { useWindowScale } from './useWindowScale';
import { PANEL_LAYOUT_KEY as LAYOUT_KEY, type PanelLayout } from './windowScale';
import AboutDialog, { type AboutDialogInfo } from './dialogs/AboutDialog';
import LicenseDialog from './dialogs/LicenseDialog';
import AgreementDialog from './dialogs/AgreementDialog';
import ImportDialog from './dialogs/ImportDialog';
import RepairDialog from './dialogs/RepairDialog';
import Jx3ColumnDialog from './dialogs/Jx3ColumnDialog';
import TourCatalogDialog from './dialogs/TourCatalogDialog';
import SettingsDialog, {
  type SettingsTab,
  type SoundSettings,
  type SoundSlot,
  type SoundSource,
} from './dialogs/SettingsDialog';
import {
  applyThemeColor,
  applyThemeMode,
  DEFAULT_THEME_COLOR,
  DEFAULT_THEME_MODE,
  SECT_PRESETS,
  type ThemeMode,
  type ThemeSettings,
} from './theme';
import UpdateNoticeDialog, {
  type UpdateNoticeInfo,
  type UpdateNoticeMode,
} from './dialogs/UpdateNoticeDialog';
import { changelogSection } from './changelog';
import { grantsSignature, type LicenseSummary, watermarkSignature } from './license';
import './PanelApp.css';
import './active-feedback.css';

/** 后端 `take_profile_notice` 的返回形态，与 `qzh_profile::SanitizeReport` 同源。 */
interface SanitizeReport {
  cleared_hotkeys: string[];
  disabled_rules: { id: string; reason: string }[];
}

/** 热键字段名 → 用户可见名称，用于净化提醒文案。 */
const HOTKEY_LABELS: Record<string, string> = {
  global_toggle: '全局开启键',
  global_stop: '全局停止键',
  panel_toggle: '面板显隐键',
};

const settingsStore = new LazyStore('settings.json');
const CLOSE_BEHAVIOR_KEY = 'closeBehavior';
/** D10：某组变成长按与切换混排时的插队提示，勾选「不再提示」后写 true。 */
const HOLD_IN_GROUP_HINT_KEY = 'holdInGroupHintDismissed';
/** 上次兑换时同意过的会员服务协议版本，再兑换时版本没变就默认勾选。 */
const MEMBERSHIP_AGREED_KEY = 'membershipAgreedVersion';
const SOUND_KEY = 'sound';
const THEME_KEY = 'theme';
// 与后端 bootstrap/update.rs 的 AUTO_UPDATE_KEY 同名，双方读同一份 settings.json
const AUTO_UPDATE_KEY = 'autoUpdate';
// 与后端 bootstrap/startup.rs 的 AUTO_ENABLE_ON_START_KEY 同名
const AUTO_ENABLE_ON_START_KEY = 'autoEnableOnStart';
const VOICES_WAIT_MS = 1500;

// WebView2 异步枚举系统语音，启动头一下 getVoices() 常是空的，这时朗读会落回默认语音而不是
// 用户选的那个。等到语音就绪或超时，超时就照默认语音播。
function waitForVoices(timeoutMs: number): Promise<void> {
  if (!('speechSynthesis' in window)) return Promise.resolve();
  const synth = window.speechSynthesis;
  if (synth.getVoices().length > 0 || typeof synth.addEventListener !== 'function') {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    const done = () => {
      clearTimeout(timer);
      synth.removeEventListener('voiceschanged', done);
      resolve();
    };
    const timer = setTimeout(done, timeoutMs);
    synth.addEventListener('voiceschanged', done);
  });
}

const DEFAULT_THEME: ThemeSettings = {
  color: DEFAULT_THEME_COLOR,
  mode: DEFAULT_THEME_MODE,
};

const DEFAULT_SOUND: SoundSettings = {
  enabled: false,
  startEnabled: true,
  endEnabled: true,
  toggleStartEnabled: false,
  toggleEndEnabled: false,
  volume: 80,
  rate: 0,
  pitch: 0,
  startText: '我准备好库库按了',
  endText: '我累了歇会',
  toggleStartText: '${key}开始',
  toggleEndText: '停止${key}',
  startSource: 'tts',
  endSource: 'tts',
  toggleStartSource: 'tts',
  toggleEndSource: 'tts',
  startAudio: '',
  endAudio: '',
  toggleStartAudio: '',
  toggleEndAudio: '',
  voiceName: '',
  globalOnly: false,
};

// 四个播报时机的字段平铺在 SoundSettings 上，集中在此表里读写，
// 免得各处手拼 `${slot}Audio` 这类字段名而丢掉类型检查。
const SOUND_SLOTS: Record<
  SoundSlot,
  {
    read: (s: SoundSettings) => { source: SoundSource; audio: string; text: string };
    pickAudio: (name: string) => Partial<SoundSettings>;
  }
> = {
  start: {
    read: (s) => ({ source: s.startSource, audio: s.startAudio, text: s.startText }),
    pickAudio: (name) => ({ startAudio: name, startSource: 'audio' }),
  },
  end: {
    read: (s) => ({ source: s.endSource, audio: s.endAudio, text: s.endText }),
    pickAudio: (name) => ({ endAudio: name, endSource: 'audio' }),
  },
  toggleStart: {
    read: (s) => ({
      source: s.toggleStartSource,
      audio: s.toggleStartAudio,
      text: s.toggleStartText,
    }),
    pickAudio: (name) => ({ toggleStartAudio: name, toggleStartSource: 'audio' }),
  },
  toggleEnd: {
    read: (s) => ({ source: s.toggleEndSource, audio: s.toggleEndAudio, text: s.toggleEndText }),
    pickAudio: (name) => ({ toggleEndAudio: name, toggleEndSource: 'audio' }),
  },
};

const SOUND_FILE_EXTENSIONS = [
  'mp3',
  'wav',
  'ogg',
  'oga',
  'opus',
  'm4a',
  'aac',
  'flac',
  'weba',
  'webm',
];
const DEFAULT_PROFILE_NAME = 'defaults';
// 注入周期基础下限 10ms（≈100 taps/s）：管线每事件过路税决定可持续「总」注入速率。单规则用
// 此值；多条规则同时连发时后端按活跃规则数等分总速率（见 burst-engine process_due），避免叠加
// 超发导致停止「收不住」。与后端 MIN_EFFECTIVE_INTERVAL_MS 对齐。
const MIN_INTERVAL_MS = 10;
const DEFAULT_INTERVAL_MS = 10;
const MAX_INTERVAL_MS = 10000;

type BurstMode = 'hold' | 'toggle';
type RuleFilter = 'all' | BurstMode;
type InputMode = 'sendinput' | 'interception' | 'ddsimple' | 'hidstick';
type DriverStatus = 'installed' | 'pending_reboot' | 'not_installed';

interface AppStatus {
  elevated: boolean;
  interception_installed: DriverStatus;
  interception_issue: string | null;
  dd_hid_installed: DriverStatus;
  hid_stick_present: boolean;
  input_mode: string;
  configured_input_mode: string;
  scheduler_hp_degraded: boolean;
  platform: string;
  os_family: string;
  os_version: string;
  webview_version: string;
  arch: string;
  locale: string;
  install_path: string;
  log_dir: string;
  app_data_dir: string;
  autostart_enabled: boolean;
  run_as_admin: boolean;
  resources_ok: boolean;
  missing_resources: string[];
}

const APP_STATUS_EVENT = 'app-status-changed';
const UPDATE_PROGRESS_EVENT = 'update-download-progress';
const UPDATE_FAILED_EVENT = 'update-download-failed';

interface UpdateDownloadFailed {
  version: string;
  message: string;
}

const INPUT_MODE_LABELS: Record<InputMode, string> = {
  sendinput: '通用模式',
  interception: '游戏模式',
  ddsimple: 'DD驱动',
  hidstick: '外设模式',
};
const INPUT_MODE_LIST: InputMode[] = ['sendinput', 'interception', 'ddsimple', 'hidstick'];
function isInputMode(value: string): value is InputMode {
  return (INPUT_MODE_LIST as string[]).includes(value);
}

function inputModeRequiresAdmin(mode: InputMode): boolean {
  return mode === 'interception' || mode === 'ddsimple';
}

// 外设模式仍限制同键切换连发，与横版键鼠图互斥。
// 已在横版时的拦截只能看目标模式名——切过去之前拿不到它的能力位。
function blocksHorizontalLayout(mode: InputMode): boolean {
  return mode === 'hidstick';
}

interface BurstRule {
  id: string;
  enabled: boolean;
  trigger_key: KeyId;
  target_key: KeyId;
  mode: BurstMode;
  stop_key: KeyId | null;
  interval_ms: number;
  group: string | null;
}

interface ProfileMeta {
  name: string;
  created_at: number;
  updated_at: number;
  app_version: string;
  signature?: string;
}

interface Profile {
  schema_version: number;
  meta: ProfileMeta;
  rules: BurstRule[];
  hotkeys: { global_toggle: KeyId | null; global_stop?: KeyId | null; panel_toggle?: KeyId | null };
  advanced: { log_level: string };
}

interface ProfileSummary {
  rules_total: number;
  rules_enabled: number;
  hold_count: number;
  toggle_count: number;
  group_count: number;
  global_toggle: KeyId | null;
  global_stop: KeyId | null;
  panel_toggle: KeyId | null;
}

interface ProfileEntry {
  meta: ProfileMeta;
  path: string;
  summary: ProfileSummary;
}

interface ForkResult {
  profile: Profile;
  path: string;
}

function newRule(mode: BurstMode = 'hold'): BurstRule {
  const isHold = mode === 'hold';
  const triggerVk = isHold ? 0x51 : 0x46; // Hold: Q, Toggle: F
  const targetVk = isHold ? 0x51 : 0x51; // 默认连发 Q；切换模式用 F 启停。
  return {
    id: crypto.randomUUID(),
    enabled: !isHold,
    trigger_key: keyboardKey(triggerVk),
    target_key: keyboardKey(targetVk),
    mode,
    stop_key: null,
    interval_ms: DEFAULT_INTERVAL_MS,
    group: null,
  };
}

function defaultRules(): BurstRule[] {
  return [newRule('hold'), newRule('toggle')];
}

/** 横版单键规则：触发键 == 连发键（== 停止键，toggle 时）。 */
function newSingleKeyRule(key: KeyId, mode: BurstMode, interval: number): BurstRule {
  return {
    id: crypto.randomUUID(),
    enabled: true,
    trigger_key: key,
    target_key: key,
    mode,
    stop_key: mode === 'toggle' ? key : null,
    interval_ms: interval,
    group: null,
  };
}

function buildProfileMenu(args: {
  profiles: ProfileEntry[];
  activeName: string;
  onSwitch: (path: string) => void;
  onCreate: () => void;
  onManage: () => void;
}): ContextMenuItem[] {
  const { profiles, activeName, onSwitch, onCreate, onManage } = args;
  const items: ContextMenuItem[] = profiles.map((p) => ({
    label: p.meta.name === DEFAULT_PROFILE_NAME ? '默认配置' : p.meta.name,
    subtitle: p.meta.name === DEFAULT_PROFILE_NAME ? '出厂预设，修改时自动新建' : undefined,
    active: p.meta.name === activeName,
    onClick: () => onSwitch(p.path),
  }));
  if (items.length > 0) items.push({ type: 'divider' });
  items.push({ label: '新建配置…', onClick: onCreate });
  items.push({ label: '管理配置…', subtitle: '重命名、删除、导入', onClick: onManage });
  return items;
}

function orderProfiles(profiles: ProfileEntry[]): ProfileEntry[] {
  return [...profiles].sort((a, b) => {
    const aDefault = a.meta.name === DEFAULT_PROFILE_NAME;
    const bDefault = b.meta.name === DEFAULT_PROFILE_NAME;
    if (aDefault !== bDefault) return aDefault ? -1 : 1;
    if (a.meta.created_at !== b.meta.created_at) return a.meta.created_at - b.meta.created_at;
    return a.meta.name.localeCompare(b.meta.name, 'zh-Hans-CN');
  });
}

export default function PanelApp() {
  const [showAgreement, setShowAgreement] = useState(false);
  const [agreementChecked, setAgreementChecked] = useState(false);
  const importWelcome = useImportWelcome(settingsStore);
  const importWelcomeHandled = useRef(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>('general');
  const [showAbout, setShowAbout] = useState(false);
  const [showLicense, setShowLicense] = useState(false);
  const [showTourCatalog, setShowTourCatalog] = useState(false);
  const [showJx3, setShowJx3] = useState(false);
  const [activeTour, setActiveTour] = useState<TourDef | undefined>(undefined);
  // 首启判定只准跑一次：版本位要等教程走完（或退出）才落盘，期间 effect 会因其它依赖重跑，
  // 没有这道闸就会二次触发。
  const firstRunHandled = useRef(false);
  // 连发按键录入计数，教程实操判定用（见 TourSnapshot.keyCaptureSeq）
  const [keyCaptureSeq, setKeyCaptureSeq] = useState(0);
  const tourProgress = useTourProgress(settingsStore);
  // 启动期两条可能弹确认框的链，各自结束（含用户点掉弹窗）时置位；没弹的也会立即置位。
  const [noticeSettled, setNoticeSettled] = useState(false);
  const [startupModeSettled, setStartupModeSettled] = useState(false);
  const [showRepair, setShowRepair] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [appVersion, setAppVersion] = useState('');
  // 更新的三种状态互不重叠：available=查到但没下，ready=下好待装，notice=弹窗当前展示的内容
  const [updateAvailable, setUpdateAvailable] = useState<UpdateNoticeInfo | null>(null);
  const [updateReady, setUpdateReady] = useState<UpdateNoticeInfo | null>(null);
  const [updateNotice, setUpdateNotice] = useState<{
    info: UpdateNoticeInfo;
    mode: UpdateNoticeMode;
  } | null>(null);
  const [applyingUpdate, setApplyingUpdate] = useState(false);
  const [autoUpdate, setAutoUpdate] = useState(true);
  const [updateProgress, setUpdateProgress] = useState<UpdateDownloadProgress | null>(null);
  const menuBtnRef = useRef<HTMLButtonElement>(null);
  const [globalEnabled, setGlobalEnabled] = useState(false);
  const globalEnabledRef = useRef(false);

  function applyGlobalEnabled(enabled: boolean) {
    globalEnabledRef.current = enabled;
    setGlobalEnabled(enabled);
  }
  const [togglingGlobal, setTogglingGlobal] = useState(false);
  const [closeBehaviorPreference, setCloseBehaviorPreference] = useState<CloseBehavior | null>(
    null,
  );
  const [inputMode, setInputMode] = useState<InputMode>('sendinput');
  const [interceptionInstalled, setInterceptionInstalled] = useState<DriverStatus>('not_installed');
  const [interceptionIssue, setInterceptionIssue] = useState<string | null>(null);
  const [ddHidInstalled, setDdHidInstalled] = useState<DriverStatus>('not_installed');
  const [hidStickPresent, setHidStickPresent] = useState(false);
  // 插拔事件在挂载期注册，要调到最新渲染的处理函数
  const hidStickConnectedRef = useRef<() => Promise<void>>(async () => {});
  const hidStickDisconnectedRef = useRef<() => Promise<void>>(async () => {});
  // 「检测到外设驱动」确认框开着时不再叠一个（启动自查与连上事件可能前后脚）
  const stickPromptingRef = useRef(false);
  const startupStickCheckedRef = useRef(false);
  // 连上时正开着别的确认框：confirm 是单实例，再弹会把那个框当「取消」顶掉，先记下等它关了再问
  const stickPromptDeferredRef = useRef(false);
  // 插入确认框 await 期间可能已重渲染，确认后要用最新的 selectInputMode
  const selectInputModeRef = useRef<(target: InputMode) => Promise<void>>(async () => {});
  const [elevated, setElevated] = useState(false);
  const [sysInfo, setSysInfo] = useState<{
    platform: string;
    os_family: string;
    os_version: string;
    webview_version: string;
    arch: string;
    locale: string;
    install_path: string;
    log_dir: string;
    app_data_dir: string;
    autostart_enabled: boolean;
    run_as_admin: boolean;
    resources_ok: boolean;
    missing_resources: string[];
    scheduler_hp_degraded: boolean;
  }>({
    platform: '',
    os_family: '',
    os_version: '',
    webview_version: '',
    arch: '',
    locale: '',
    install_path: '',
    log_dir: '',
    app_data_dir: '',
    autostart_enabled: false,
    run_as_admin: false,
    resources_ok: true,
    missing_resources: [],
    scheduler_hp_degraded: false,
  });
  const [togglingAutostart, setTogglingAutostart] = useState(false);
  const [togglingRunAsAdmin, setTogglingRunAsAdmin] = useState(false);
  const [autoEnableOnStart, setAutoEnableOnStart] = useState(false);
  const [sound, setSound] = useState<SoundSettings>(DEFAULT_SOUND);
  const soundRef = useRef<SoundSettings>(DEFAULT_SOUND);
  const [soundSettled, setSoundSettled] = useState(false);
  // checked：启动收尾那一问已经问过；enabled：本进程已按设置开过，同意协议后的补问不再重复开
  const startupAutoEnableCheckedRef = useRef(false);
  const startupAutoEnabledRef = useRef(false);
  const audioUrlCache = useRef(new Map<string, string>());
  const audioElRef = useRef<HTMLAudioElement | null>(null);
  const playbackSeq = useRef(0);
  const [theme, setTheme] = useState<ThemeSettings>(DEFAULT_THEME);
  const themeModeRef = useRef<ThemeMode>(DEFAULT_THEME.mode);
  const [availableVoices, setAvailableVoices] = useState<string[]>([]);
  const [switchingMode, setSwitchingMode] = useState(false);
  const [modePickerOpen, setModePickerOpen] = useState(false);
  const modeBtnRef = useRef<HTMLButtonElement>(null);
  const [rules, setRules] = useState<BurstRule[]>([]);
  const rulesRef = useRef(rules);
  const profileAdvancedRef = useRef<Profile['advanced']>({ log_level: 'info' });
  // 全局开关启用时轮询，驱动激活态光晕；关闭时清空，避免残留高亮。
  const { active: activeRuleIds, paused: pausedRuleIds } = useRuleStates(globalEnabled, 120);
  const [profileName, setProfileName] = useState('defaults');
  const [profileList, setProfileList] = useState<ProfileEntry[]>([]);
  const [profileMenuOpen, setProfileMenuOpen] = useState(false);
  const profileBtnRef = useRef<HTMLButtonElement>(null);
  const [advancedOpen, setAdvancedOpen] = useState<Record<string, boolean>>({});
  const [draggingId, setDraggingId] = useState<string | undefined>(undefined);
  const draggingIdRef = useRef<string | undefined>(undefined);
  const [dragOverInfo, setDragOverInfo] = useState<
    | { kind: 'rule'; ruleId: string }
    | { kind: 'group'; name: string }
    | { kind: 'ungrouped' }
    | undefined
  >(undefined);
  const [pendingGroupName, setPendingGroupName] = useState<string | null>(null);
  const [editingGroupName, setEditingGroupName] = useState<{
    current: string;
    draft: string;
  } | null>(null);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  // 筛选只影响显示；不持久化，免得下次打开时规则被上次的筛选藏起来像是丢了。
  const [filter, setFilter] = useState<RuleFilter>('all');
  // 面板布局：竖版规则列表 / 横版键鼠图，各有设计尺寸与记住的缩放比例，切换时定尺寸并居中。
  const [layout, setLayout] = useState<PanelLayout>('vertical');
  const {
    scale: windowScale,
    maximized,
    applyLayout,
    resetScale,
    toggleMaximize,
    dragFromMaximized,
  } = useWindowScale(settingsStore);

  /**
   * 标题栏自己处理按下，不用 data-tauri-drag-region：最大化是自己模拟的，内置拖动区的双击会走系统最大化、
   * 拖动也不会先还原。双击切换；最大化时按住拖出几像素才还原，单击不动。
   */
  function handleHeaderMouseDown(e: ReactMouseEvent) {
    if (e.button !== 0 || (e.target as Element).closest('button, a, input')) return;
    if (e.detail === 2) {
      void toggleMaximize();
      return;
    }
    if (!maximized) {
      getCurrentWindow()
        .startDragging()
        .catch(() => {});
      return;
    }
    const { screenX: x0, screenY: y0 } = e;
    const stop = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', stop);
    };
    const onMove = (ev: MouseEvent) => {
      if (Math.abs(ev.screenX - x0) + Math.abs(ev.screenY - y0) < 4) return;
      stop();
      void dragFromMaximized();
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', stop);
  }
  // 横版统一间隔的回退值（无规则时显示用；有规则时显示取自规则本身）
  const [unifiedInterval, setUnifiedInterval] = useState(DEFAULT_INTERVAL_MS);
  const [hotkeys, setHotkeys] = useState<{
    global_toggle: KeyId | null;
    global_stop: KeyId | null;
    panel_toggle: KeyId | null;
  }>({ global_toggle: null, global_stop: null, panel_toggle: null });
  const hotkeysRef = useRef(hotkeys);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const conflicts = useMemo(() => detectConflicts(rules, hotkeys), [rules, hotkeys]);
  const confirm = useConfirm();
  const confirmOpen = useConfirmOpen();
  const toast = useToast();

  // 按键允许集来自后端（`packages/qzh-profile/src/key_policy.rs` 是唯一的表）。
  // 后端未答复前用最保守的兜底，避免这一小段时间里放行了当前模式其实不支持的键。
  const [keyPolicies, setKeyPolicies] = useState<KeyPolicies>({
    hotkey: RESTRICTIVE_POLICY,
    trigger: RESTRICTIVE_POLICY,
    target: RESTRICTIVE_POLICY,
    trigger_target: RESTRICTIVE_POLICY,
    trigger_target_toggle: RESTRICTIVE_POLICY,
    coincident_toggle: true,
  });

  useEffect(() => {
    invoke<KeyPolicies>('get_key_policy')
      .then(setKeyPolicies)
      .catch(() => {});
  }, [inputMode]);

  // 连发规则不开放修饰键（见 KeyCapture 的 MODIFIER_VK 注释）；连同查表落空的键一起提示，
  // 否则这次按键被静默丢弃、用户无从判断原因。稳定引用避免 KeyCapture 每次渲染重挂监听。
  const notifyRuleKeyReject = useCallback(
    (info: CaptureReject) => {
      if (info.reason === 'modifier') {
        toast.warning('修饰键只能绑定全局热键，连发规则请换其它按键');
        return;
      }
      if (info.reason === 'unknown') {
        toast.warning(`不支持绑定这个按键（${info.code}），请换一个`);
        return;
      }
      if (info.reason === 'mouse-unsupported') {
        toast.warning(
          `当前输入模式无法注入${keyLabel(mouseKey(info.button))}，请换其它按键或切换输入模式`,
        );
        return;
      }
      if (info.reason === 'slot-disabled') {
        toast.warning(
          `${INPUT_MODE_LABELS[inputMode]}不支持切换连发的启动键与连发按键相同，请在高级设置里把两者分开`,
        );
        return;
      }
      toast.warning('这个位置只支持键盘按键，请换一个');
    },
    [toast, inputMode],
  );
  const profilePersistenceRef = useRef<
    ReturnType<typeof createProfilePersistence<Profile>> | undefined
  >(undefined);
  if (!profilePersistenceRef.current) {
    profilePersistenceRef.current = createProfilePersistence<Profile>(
      async (name, profile) => {
        if (name === DEFAULT_PROFILE_NAME) throw new Error('默认配置另存失败，未覆盖默认配置');
        await invoke('save_profile', {
          name,
          profile: { ...profile, meta: { ...profile.meta, name } },
        });
      },
      () => toast.warning('自动保存配置失败，请重试后再切换配置'),
    );
  }
  const profilePersistence = profilePersistenceRef.current;
  const updateProgressDoneTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const updateDownloadFailedRef = useRef(false);
  const initialLoadDone = useRef(false);
  // 与 initialLoadDone 同步的 state 版：ref 置位不会触发重渲染，首启教程那条 effect 需要一个
  // 能进依赖数组的「配置已装载」信号。（曾经靠 rules 变化顺带重跑，删掉 rules 依赖后教程就再也不弹了。）
  const [initialLoadSettled, setInitialLoadSettled] = useState(false);
  const startupInputModeHandledRef = useRef(false);
  const profileNameRef = useRef(profileName);
  // 供挂载期注册的事件监听调用最新的 switchToProfile（避免闭包锁死首帧版本）
  const switchToProfileRef = useRef<(path: string) => Promise<void>>(async () => {});
  // WebView2 聚焦时全局键盘钩子失效，将键盘事件中继到后端引擎（与浮窗共用）。
  useKeyRelay();
  const isDefaultProfile = profileName === DEFAULT_PROFILE_NAME;

  const [license, setLicense] = useState<LicenseSummary | undefined>(undefined);
  const [agreedVersion, setAgreedVersion] = useState<string | undefined>(undefined);
  useEffect(() => {
    invoke<LicenseSummary>('get_license_status')
      .then(setLicense)
      .catch(() => {});
    settingsStore
      .get<string>(MEMBERSHIP_AGREED_KEY)
      .then((v) => setAgreedVersion(v ?? undefined))
      .catch(() => {});
  }, []);
  const activeSignature = watermarkSignature(license, profileList, profileName);

  // 加载配置时被净化掉的项，取走一次就弹一次。启动加载先于窗口创建，事件会丢，
  // 故后端把结果存在 ProfileNotice 里等前端来取；运行期切换配置再靠事件触发这里重取。
  const showProfileNotice = useCallback(async () => {
    const notice = await invoke<SanitizeReport | null>('take_profile_notice').catch(() => null);
    if (!notice) return;
    const lines: string[] = [];
    if (notice.cleared_hotkeys.length > 0) {
      const names = notice.cleared_hotkeys.map((f) => HOTKEY_LABELS[f] ?? f).join('、');
      lines.push(`${names}原先绑定的按键当前版本不支持，已清除，请重新设置。`);
    }
    if (notice.disabled_rules.length > 0) {
      lines.push(
        `${notice.disabled_rules.length} 条规则用到了当前输入模式不支持的按键，已暂时停用。` +
          '按键没有改动，改成支持的键后重新勾选启用即可。',
      );
    }
    await confirm({
      title: '部分配置已调整',
      description: lines.join('\n'),
      confirmText: '知道了',
      cancelText: null,
    });
  }, [confirm]);

  useEffect(() => {
    void showProfileNotice().finally(() => setNoticeSettled(true));
    const un = listen('profile-sanitized', () => {
      void showProfileNotice();
    });
    return () => {
      void un.then((f) => f());
    };
  }, [showProfileNotice]);

  const applyAppStatus = useCallback((status: AppStatus) => {
    setElevated(status.elevated);
    setInterceptionInstalled(status.interception_installed);
    setInterceptionIssue(status.interception_issue);
    setDdHidInstalled(status.dd_hid_installed);
    setHidStickPresent(status.hid_stick_present);
    setSysInfo({
      platform: status.platform,
      os_family: status.os_family,
      os_version: status.os_version,
      webview_version: status.webview_version,
      arch: status.arch,
      locale: status.locale,
      install_path: status.install_path,
      log_dir: status.log_dir,
      app_data_dir: status.app_data_dir,
      autostart_enabled: status.autostart_enabled,
      run_as_admin: status.run_as_admin,
      resources_ok: status.resources_ok,
      missing_resources: status.missing_resources,
      scheduler_hp_degraded: status.scheduler_hp_degraded,
    });
    if (isInputMode(status.input_mode)) {
      setInputMode(status.input_mode);
    }
  }, []);

  async function reconcileStartupInputMode(status: AppStatus) {
    if (startupInputModeHandledRef.current) return;
    if (!isInputMode(status.configured_input_mode)) return;

    const configured = status.configured_input_mode;
    if (configured === 'sendinput' || configured === status.input_mode) return;

    startupInputModeHandledRef.current = true;

    if (configured === 'interception' && status.elevated && status.interception_issue) {
      await showInterceptionIssue(status.interception_issue);
      return;
    }

    if (inputModeRequiresAdmin(configured) && !status.elevated) {
      const label = INPUT_MODE_LABELS[configured];
      const ok = await confirm({
        title: `以管理员模式恢复${label}`,
        description: (
          <>
            已保存的输入模式是{label}，需要管理员权限才能初始化驱动通道。当前将先使用通用模式；
            授权后应用会以管理员权限重启，并自动切换到{label}。
          </>
        ),
        confirmText: '以管理员重启',
        cancelText: '保持通用模式',
      });
      if (!ok) return;
      try {
        await invoke('relaunch_as_admin', { mode: configured });
      } catch (e) {
        toast.error(`启动管理员实例失败：${e}`);
      }
      return;
    }

    try {
      await invoke('set_input_mode', { mode: configured });
      const nextStatus = await invoke<AppStatus>('get_app_status');
      applyAppStatus(nextStatus);
      if (nextStatus.input_mode === configured) {
        toast.success(`已恢复为${INPUT_MODE_LABELS[configured]}`);
      } else if (isInputMode(nextStatus.input_mode)) {
        toast.warning(`未能恢复配置模式，已停留在${INPUT_MODE_LABELS[nextStatus.input_mode]}`);
      }
    } catch (e) {
      toast.error(`恢复配置模式失败：${e}`);
    }
  }

  useEffect(() => {
    return () => {
      profilePersistence.dispose();
      if (updateProgressDoneTimer.current) clearTimeout(updateProgressDoneTimer.current);
    };
  }, []);

  useEffect(() => {
    getVersion()
      .then(setAppVersion)
      .catch(() => {});
  }, []);

  // 首启引导（D8）：按 TOUR_INTRO_VERSION 判定，与用户协议同一套语义——存的版本对不上就自动跑
  // 一遍「上手三步」。自动开的那一组由 handleTourExit 负责记版本，否则中途退出就再也不会弹。
  // 就绪门只挡这条自动触发：菜单入口不挡，启动头两秒点了没反应比气泡晚两秒更费解，
  // 手动开的教程遇到确认框由 paused 让路即可。
  useEffect(() => {
    if (!importWelcome.loaded || !importWelcome.pending || importWelcomeHandled.current) return;
    if (
      !agreementChecked ||
      !initialLoadSettled ||
      showAgreement ||
      !noticeSettled ||
      !startupModeSettled
    )
      return;
    if (confirmOpen || updateNotice !== null || showImport) return;
    importWelcomeHandled.current = true;
    void (async () => {
      const scan = await confirm({
        title: '要导入已有的按键配置吗？',
        description:
          '支持气质花 .qzh，以及丐帮高手、衍天高手、极简按键和小叽按键配置。可以先扫描，再自行选择配置、查看预览，确认后才导入。以后也能从「☰ → 导入配置」打开。',
        confirmText: '扫描配置',
        cancelText: '暂不导入',
      });
      if (scan) setShowImport(true);
      await importWelcome
        .complete()
        .catch(() => toast.warning('导入提示状态未能保存，本次选择仍然有效'));
    })();
  }, [
    importWelcome.loaded,
    importWelcome.pending,
    agreementChecked,
    initialLoadSettled,
    showAgreement,
    noticeSettled,
    startupModeSettled,
    confirmOpen,
    updateNotice,
    showImport,
  ]);

  useEffect(() => {
    if (!tourProgress.loaded || tourProgress.introSeen || firstRunHandled.current) return;
    if (!agreementChecked || !importWelcome.loaded || importWelcome.pending || showImport) return;
    if (!initialLoadSettled || showAgreement) return;
    if (!noticeSettled || !startupModeSettled) return;
    // 启动期的自动更新检查是后台跑的，update-ready 可能在任何时刻弹。它开着时不能 startTour，
    // 否则日志会记一次假的 started。update-available 只亮标题栏、update-downloading 只画进度条，
    // 都不打断用户，不进门。
    if (updateNotice !== null || applyingUpdate) return;
    firstRunHandled.current = true;
    const intro = findTour('getting-started');
    if (intro) startTour(intro);
    else tourProgress.markIntroSeen().catch(() => {});
    // startTour / markIntroSeen 同样是渲染期新建的函数，纳入依赖会让 effect 每次渲染重跑
  }, [
    tourProgress.loaded,
    tourProgress.introSeen,
    agreementChecked,
    importWelcome.loaded,
    importWelcome.pending,
    showImport,
    initialLoadSettled,
    showAgreement,
    noticeSettled,
    startupModeSettled,
    updateNotice,
    applyingUpdate,
  ]);

  useEffect(() => {
    settingsStore
      .get<boolean>(AUTO_UPDATE_KEY)
      .then((v) => {
        if (typeof v === 'boolean') setAutoUpdate(v);
      })
      .catch(() => {});
    settingsStore
      .get<boolean>(AUTO_ENABLE_ON_START_KEY)
      .then((v) => {
        if (typeof v === 'boolean') setAutoEnableOnStart(v);
      })
      .catch(() => {});
  }, []);

  // 主题：从 store 加载主色 + 亮暗模式并立即应用；mode=system 时监听系统配色变化。
  useEffect(() => {
    settingsStore
      .get<ThemeSettings>(THEME_KEY)
      .then((v) => {
        const merged: ThemeSettings = { ...DEFAULT_THEME, ...v };
        setTheme(merged);
        themeModeRef.current = merged.mode;
        applyThemeColor(merged.color);
        applyThemeMode(merged.mode);
      })
      .catch(() => {});

    const mql = window.matchMedia('(prefers-color-scheme: dark)');
    const onSystemChange = () => {
      if (themeModeRef.current === 'system') applyThemeMode('system');
    };
    mql.addEventListener('change', onSystemChange);
    return () => mql.removeEventListener('change', onSystemChange);
  }, []);

  // 声音设置：从 store 加载，同时枚举系统语音列表
  useEffect(() => {
    settingsStore
      .get<SoundSettings>(SOUND_KEY)
      .then((v) => {
        if (v && typeof v === 'object') {
          const merged = { ...DEFAULT_SOUND, ...v };
          setSound(merged);
          soundRef.current = merged;
        }
      })
      .catch(() => {})
      .finally(() => setSoundSettled(true));

    if (!('speechSynthesis' in window)) return;

    const speechSynthesis = window.speechSynthesis;
    const loadVoices = () => {
      const voices = speechSynthesis
        .getVoices()
        .filter((v) => v.lang.startsWith('zh') || v.lang.startsWith('en'))
        .map((v) => v.name);
      if (voices.length > 0) setAvailableVoices(voices);
    };
    loadVoices();

    if (
      typeof speechSynthesis.addEventListener === 'function' &&
      typeof speechSynthesis.removeEventListener === 'function'
    ) {
      speechSynthesis.addEventListener('voiceschanged', loadVoices);
      return () => speechSynthesis.removeEventListener('voiceschanged', loadVoices);
    }

    const previousVoicesChanged = speechSynthesis.onvoiceschanged;
    speechSynthesis.onvoiceschanged = (event) => {
      previousVoicesChanged?.call(speechSynthesis, event);
      loadVoices();
    };
    return () => {
      speechSynthesis.onvoiceschanged = previousVoicesChanged;
    };
  }, []);

  // 卸载时释放自选音频的 Blob URL，避免窗口重建后旧对象常驻内存
  useEffect(() => {
    const cache = audioUrlCache.current;
    return () => {
      for (const url of cache.values()) URL.revokeObjectURL(url);
      cache.clear();
    };
  }, []);

  useEffect(() => {
    settingsStore
      .get<CloseBehavior>(CLOSE_BEHAVIOR_KEY)
      .then((v) => {
        setCloseBehaviorPreference(v === 'exit' || v === 'minimize' ? v : null);
      })
      .catch(() => {});

    settingsStore
      .get<PanelLayout>(LAYOUT_KEY)
      .then((v) => {
        // 没存过布局也要走一遍：竖版可能存过放大比例
        const initial = v === 'horizontal' ? 'horizontal' : 'vertical';
        setLayout(initial);
        void applyLayout(initial);
      })
      .catch(() => {
        void applyLayout('vertical');
      });

    invoke<boolean>('get_global_enabled')
      .then(applyGlobalEnabled)
      .catch(() => {
        toast.error('读取全局开关状态失败');
      });

    invoke<AppStatus>('get_app_status')
      .then((status) => {
        applyAppStatus(status);
        void reconcileStartupInputMode(status).finally(() => setStartupModeSettled(true));
      })
      // 取状态失败也要放行，否则这条链永远不结束、首启教程再也不会出现
      .catch(() => setStartupModeSettled(true));

    // 启动期：以「activeProfilePath → load_profile」为唯一来源；
    // 没路径或加载失败则回退到 init_default_profile。
    void (async () => {
      const refreshList = async () => {
        try {
          const list = await invoke<ProfileEntry[]>('list_profiles');
          setProfileList(orderProfiles(list));
        } catch {
          /* 启动时静默失败，后续操作再提示 */
        }
      };
      try {
        const activePath = await invoke<string | null>('get_active_profile_path');
        if (activePath) {
          try {
            const profile = await invoke<Profile>('load_profile', { path: activePath });
            applyLoadedProfile(profile);
            await refreshList();
            queueMicrotask(() => {
              initialLoadDone.current = true;
              setInitialLoadSettled(true);
            });
            return;
          } catch {
            toast.warning('加载配置失败，已切换为默认配置');
          }
        }
        const profile = await invoke<Profile>('init_default_profile');
        applyLoadedProfile(profile);
        await refreshList();
      } catch {
        toast.error('初始化默认配置失败');
        setRules(defaultRules());
        invoke('set_rules', { rules: defaultRules() }).catch(() => {});
      } finally {
        queueMicrotask(() => {
          initialLoadDone.current = true;
          setInitialLoadSettled(true);
        });
      }
    })();

    const unlistenAgreement = listen<string>('show-agreement', () => {
      setShowAgreement(true);
    });
    // 兜底：如果 emit 在 listen 注册前就已触发（WebView 加载慢时可能丢失事件）
    invoke<boolean>('needs_agreement')
      .then((needed) => {
        if (needed) setShowAgreement(true);
      })
      .catch(() => setShowAgreement(true))
      .finally(() => setAgreementChecked(true));
    const unlistenStatus = listen<AppStatus>(APP_STATUS_EVENT, (e) => {
      applyAppStatus(e.payload);
    });
    const unlistenGlobal = listen<boolean>('global-enabled-changed', (e) => {
      applyGlobalEnabled(e.payload);
    });
    const unlistenStickConnected = listen('hid-stick-connected', () => {
      void hidStickConnectedRef.current();
    });
    const unlistenStickDisconnected = listen('hid-stick-disconnected', () => {
      void hidStickDisconnectedRef.current();
    });
    const unlistenDownloading = listen<{ version: string; silent: boolean }>(
      'update-downloading',
      (e) => {
        if (updateProgressDoneTimer.current) clearTimeout(updateProgressDoneTimer.current);
        updateDownloadFailedRef.current = false;
        setUpdateProgress({
          version: e.payload.version,
          downloaded: 0,
          total: null,
          percent: null,
          done: false,
        });
        // 启动期自动下载只画进度条，不弹 toast 抢注意力
        if (!e.payload.silent) toast.info(`发现新版本 v${e.payload.version}，正在下载更新…`);
      },
    );
    const unlistenProgress = listen<UpdateDownloadProgress>(UPDATE_PROGRESS_EVENT, (e) => {
      if (updateProgressDoneTimer.current) clearTimeout(updateProgressDoneTimer.current);
      setUpdateProgress(e.payload);
      if (e.payload.done) {
        updateProgressDoneTimer.current = setTimeout(() => {
          setUpdateProgress((current) =>
            current?.version === e.payload.version && current.done ? null : current,
          );
        }, 1800);
      }
    });
    const unlistenFailed = listen<UpdateDownloadFailed>(UPDATE_FAILED_EVENT, (e) => {
      if (updateProgressDoneTimer.current) clearTimeout(updateProgressDoneTimer.current);
      updateDownloadFailedRef.current = true;
      setUpdateProgress(null);
      toast.warning(`下载更新失败：${e.payload.message}`);
    });
    // 自动更新关闭时后端只报「有新版本」，不下载，交给标题栏提示
    const unlistenAvailable = listen<UpdateNoticeInfo>('update-available', (e) => {
      setUpdateAvailable(e.payload);
    });
    const unlistenReady = listen<UpdateNoticeInfo>('update-ready', (e) => {
      setUpdateAvailable(null);
      setUpdateReady(e.payload);
      setUpdateNotice({ info: e.payload, mode: 'ready' });
    });
    const unlistenUpToDate = listen('update-not-available', () => {
      setUpdateProgress(null);
      toast.info('已是最新版本');
    });
    // 托盘与面板复用保存屏障，不能先激活后台再补存旧配置。
    const unlistenProfileSwitch = listen<string>('profile-switch-requested', (e) => {
      void switchToProfileRef.current(e.payload);
    });
    const unlistenClose = getCurrentWindow().onCloseRequested((event) => {
      event.preventDefault();
      void handleClose();
    });
    return () => {
      unlistenAgreement.then((fn) => fn());
      unlistenStatus.then((fn) => fn());
      unlistenGlobal.then((fn) => fn());
      unlistenStickConnected.then((fn) => fn());
      unlistenStickDisconnected.then((fn) => fn());
      unlistenDownloading.then((fn) => fn());
      unlistenProgress.then((fn) => fn());
      unlistenFailed.then((fn) => fn());
      unlistenAvailable.then((fn) => fn());
      unlistenReady.then((fn) => fn());
      unlistenUpToDate.then((fn) => fn());
      unlistenProfileSwitch.then((fn) => fn());
      unlistenClose.then((fn) => fn());
    };
  }, []);

  // 规则/热键变更后防抖自动保存到 .qzh
  profileNameRef.current = profileName;
  rulesRef.current = rules;
  hotkeysRef.current = hotkeys;
  switchToProfileRef.current = switchToProfile;
  hidStickConnectedRef.current = handleHidStickConnected;
  hidStickDisconnectedRef.current = handleHidStickDisconnected;
  selectInputModeRef.current = selectInputMode;

  const refreshProfileList = useCallback(async () => {
    try {
      const list = await invoke<ProfileEntry[]>('list_profiles');
      setProfileList(orderProfiles(list));
    } catch {
      toast.warning('读取配置列表失败');
    }
  }, [toast]);

  const forkPromiseRef = useRef<Promise<void> | null>(null);

  // ensureWritableProfile：若当前是默认配置，启动 fork；已在 fork 中则返回同一 Promise。
  // 只能由用户操作（pushRules / 热键 onChange）调用，不能放进会在 profile load 时触发的 effect。
  function ensureWritableProfile(): Promise<void> {
    if (profileNameRef.current !== DEFAULT_PROFILE_NAME) return Promise.resolve();
    if (forkPromiseRef.current) return forkPromiseRef.current;
    forkPromiseRef.current = invoke<ForkResult>('fork_active_profile', {
      suggestedName: '我的配置',
    })
      .then(async (res) => {
        setProfileName(res.profile.meta.name);
        profileNameRef.current = res.profile.meta.name;
        await refreshProfileList();
        toast.success(`已为你创建新配置「${res.profile.meta.name}」`);
      })
      .catch((e) => {
        throw new Error(`创建新配置失败：${e}`);
      })
      .finally(() => {
        forkPromiseRef.current = null;
      });
    return forkPromiseRef.current;
  }

  function captureProfileWrite(): ProfileWriteTicket {
    if (profileNameRef.current !== DEFAULT_PROFILE_NAME) {
      return profilePersistence.capture(profileNameRef.current);
    }
    return profilePersistence.capture(ensureWritableProfile().then(() => profileNameRef.current));
  }

  function saveProfile(ticket: ProfileWriteTicket) {
    profilePersistence.schedule(ticket, {
      schema_version: 4,
      meta: { name: '', created_at: 0, updated_at: 0, app_version: '' },
      rules: rulesRef.current,
      hotkeys: { ...hotkeysRef.current },
      advanced: { ...profileAdvancedRef.current },
    });
  }

  // 只在用户编辑时提交，加载配置不会反过来触发一次保存或热键写入。
  function changeHotkeys(patch: Partial<typeof hotkeys>) {
    if (profilePersistence.isChanging()) return;
    const next = { ...hotkeysRef.current, ...patch };
    const ticket = captureProfileWrite();
    hotkeysRef.current = next;
    setHotkeys(next);
    let writable = false;
    const operation = ticket.name
      .then(() => {
        writable = true;
        return invoke('set_global_hotkeys', {
          hotkeys: {
            global_toggle: next.global_toggle,
            global_stop: next.global_stop,
            panel_toggle: next.panel_toggle,
          },
        });
      })
      .then(() => {
        if (profilePersistence.isCurrent(ticket)) saveProfile(ticket);
      })
      .catch(async (e) => {
        if (!profilePersistence.isCurrent(ticket)) return;
        toast.error(`保存热键失败：${e}`);
        try {
          const engineHotkeys = await invoke<Profile['hotkeys']>('get_hotkeys');
          if (!profilePersistence.isCurrent(ticket)) return;
          const restored = {
            global_toggle: engineHotkeys.global_toggle ?? null,
            global_stop: engineHotkeys.global_stop ?? null,
            panel_toggle: engineHotkeys.panel_toggle ?? null,
          };
          hotkeysRef.current = restored;
          setHotkeys(restored);
        } catch {
          hotkeysRef.current = { global_toggle: null, global_stop: null, panel_toggle: null };
          setHotkeys(hotkeysRef.current);
        }
        if (writable) saveProfile(ticket);
      });
    profilePersistence.track(operation);
  }

  async function changeProfile<R>(action: () => Promise<R>): Promise<R> {
    try {
      return await profilePersistence.change(async () => {
        initialLoadDone.current = false;
        setInitialLoadSettled(false);
        return await action();
      });
    } finally {
      if (!profilePersistence.isChanging()) {
        queueMicrotask(() => {
          initialLoadDone.current = true;
          setInitialLoadSettled(true);
        });
      }
    }
  }

  function applyLoadedProfile(profile: Profile) {
    rulesRef.current = profile.rules;
    hotkeysRef.current = {
      global_toggle: profile.hotkeys.global_toggle ?? null,
      global_stop: profile.hotkeys.global_stop ?? null,
      panel_toggle: profile.hotkeys.panel_toggle ?? null,
    };
    profileAdvancedRef.current = profile.advanced;
    profileNameRef.current = profile.meta.name;
    setRules(profile.rules);
    setHotkeys(hotkeysRef.current);
    setProfileName(profile.meta.name);
    setAdvancedOpen({});
  }

  // 全局开关切换时播报语音；initialLoadDone 为 true 后才响应，跳过启动阶段的状态同步
  useEffect(() => {
    if (!initialLoadDone.current) return;
    playGlobalChange(globalEnabled);
  }, [globalEnabled]); // eslint-disable-line react-hooks/exhaustive-deps

  // 启动时外设驱动本就插着：连上事件可能早于面板开始监听，落定后自己查一次。等配置调整通知
  // 处理完、协议同意之后再查——前者只弹一次，被顶掉就再也看不到；后者挡着时问了也是白问
  useEffect(() => {
    if (startupStickCheckedRef.current) return;
    if (!initialLoadSettled || !startupModeSettled || !noticeSettled || showAgreement) return;
    if (!importWelcome.loaded || importWelcome.pending || showImport) return;
    startupStickCheckedRef.current = true;
    void hidStickConnectedRef.current();
  }, [
    initialLoadSettled,
    startupModeSettled,
    noticeSettled,
    showAgreement,
    importWelcome.loaded,
    importWelcome.pending,
    showImport,
  ]);

  useEffect(() => {
    if (confirmOpen || !stickPromptDeferredRef.current) return;
    if (!importWelcome.loaded || importWelcome.pending || showImport) return;
    stickPromptDeferredRef.current = false;
    void hidStickConnectedRef.current();
  }, [confirmOpen, importWelcome.loaded, importWelcome.pending, showImport]);

  // 启动后自动开全局与恢复配置模式同一思路：后端只给判定，面板走用户点开关的同一条路径打开，
  // 播报、托盘、浮窗才与手动开启一致。配置、输入模式、声音设置都落定后再开——输入模式可能
  // 还要切或弹窗问提权，声音设置没读到会按默认值播报。
  useEffect(() => {
    if (startupAutoEnableCheckedRef.current) return;
    if (!initialLoadSettled || !startupModeSettled || !soundSettled) return;
    if (
      !agreementChecked ||
      showAgreement ||
      !importWelcome.loaded ||
      importWelcome.pending ||
      showImport
    )
      return;
    startupAutoEnableCheckedRef.current = true;
    void applyStartupAutoEnable();
  }, [
    initialLoadSettled,
    startupModeSettled,
    soundSettled,
    agreementChecked,
    showAgreement,
    importWelcome.loaded,
    importWelcome.pending,
    showImport,
  ]); // eslint-disable-line react-hooks/exhaustive-deps

  // 协议没同意时后端判定为否；同意后 handleAgreed 再问一次。
  async function applyStartupAutoEnable() {
    if (startupAutoEnabledRef.current) return;
    const should = await invoke<boolean>('should_auto_enable_on_start').catch(() => false);
    if (!should || startupAutoEnabledRef.current) return;
    startupAutoEnabledRef.current = true;
    if (soundRef.current.voiceName) await waitForVoices(VOICES_WAIT_MS);
    await setGlobal(true);
  }

  // 语音跟随引擎实际启停事件；状态轮询只负责界面，避免快速启停在两次轮询之间漏掉。
  useEffect(() => {
    let disposed = false;
    const handleFeedback = createToggleFeedbackHandler({
      isReady: () => !disposed && initialLoadDone.current,
      isEnabled: () => globalEnabledRef.current,
      play: playToggleFeedback,
    });
    const unlisten = listen<ToggleFeedback>('toggle-feedback', ({ payload }) =>
      handleFeedback(payload),
    );
    return () => {
      disposed = true;
      void unlisten.then((stop) => stop());
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  function buildUtterance(text: string, s: SoundSettings): SpeechSynthesisUtterance {
    const utt = new SpeechSynthesisUtterance(text);
    utt.lang = 'zh-CN';
    utt.volume = s.volume / 100;
    // rate: -10..+10 → 0.5..1.5（中点 0 对应默认 1）
    utt.rate = 1 + s.rate * 0.05;
    // pitch: -10..+10 → 0..2（Web Speech API 原生属性，中点 0 对应默认 1）
    utt.pitch = 1 + s.pitch * 0.1;
    const voice = window.speechSynthesis.getVoices().find((v) => v.name === s.voiceName);
    if (voice) utt.voice = voice;
    return utt;
  }

  function speakText(text: string, s: SoundSettings) {
    if (!('speechSynthesis' in window)) return;
    if (text.trim().length === 0) return;
    const synth = window.speechSynthesis;
    synth.speak(buildUtterance(text, s));
    if (synth.paused) synth.resume();
  }

  // 自选音频的字节经 IPC 取回后转成 Blob URL 缓存，避免每次提示都重读一遍文件。
  function soundUrl(name: string): Promise<string> {
    const cached = audioUrlCache.current.get(name);
    if (cached) return Promise.resolve(cached);
    return invoke<ArrayBuffer>('read_sound_file', { name }).then((bytes) => {
      const url = URL.createObjectURL(new Blob([bytes]));
      audioUrlCache.current.set(name, url);
      return url;
    });
  }

  function playAudioFile(name: string, volume: number) {
    // 取字节是异步的，期间可能已经有新提示插队，用序号只放行最后一次请求
    const seq = playbackSeq.current;
    soundUrl(name)
      .then((url) => {
        if (seq !== playbackSeq.current) return;
        let el = audioElRef.current;
        if (!el) {
          el = new Audio();
          audioElRef.current = el;
        }
        if (el.src !== url) el.src = url;
        el.volume = Math.min(1, Math.max(0, volume / 100));
        el.currentTime = 0;
        void el.play().catch(() => {});
      })
      .catch(() => {
        toast.error(`提示音「${name}」播放失败`);
      });
  }

  // 朗读与音频共用「最新覆盖」语义：新提示先掐断上一条，避免连续切换时叠音。
  function stopFeedback() {
    playbackSeq.current += 1;
    if ('speechSynthesis' in window) window.speechSynthesis.cancel();
    audioElRef.current?.pause();
  }

  // text 为已展开 ${key} 的成品；音源选了音频却没选文件时静默跳过。
  function playFeedback(slot: SoundSlot, text: string, s: SoundSettings) {
    const { source, audio } = SOUND_SLOTS[slot].read(s);
    stopFeedback();
    if (source === 'audio') {
      if (audio) playAudioFile(audio, s.volume);
      return;
    }
    speakText(text, s);
  }

  function playGlobalChange(enabled: boolean) {
    const s = soundRef.current;
    if (!s.enabled) return;
    if (enabled ? !s.startEnabled : !s.endEnabled) return;
    playFeedback(enabled ? 'start' : 'end', enabled ? s.startText : s.endText, s);
  }

  function playToggleFeedback(key: KeyId, isStart: boolean) {
    const s = soundRef.current;
    if (!s.enabled) return;
    if (isStart ? !s.toggleStartEnabled : !s.toggleEndEnabled) return;
    const slot: SoundSlot = isStart ? 'toggleStart' : 'toggleEnd';
    const template = (isStart ? s.toggleStartText : s.toggleEndText) ?? '';
    playFeedback(slot, template.split('${key}').join(keyLabel(key)), s);
  }

  function previewSound(slot: SoundSlot) {
    const s = soundRef.current;
    const { text } = SOUND_SLOTS[slot].read(s);
    playFeedback(slot, text.split('${key}').join('F'), s);
  }

  // 选中的音频复制进应用数据目录后只留文件名；被换下的旧文件若已无人引用就删掉，
  // 否则反复更换会在 sounds/ 里堆垃圾。
  async function pickSoundAudio(slot: SoundSlot) {
    const path = await openFileDialog({
      multiple: false,
      filters: [{ name: '音频文件', extensions: SOUND_FILE_EXTENSIONS }],
    });
    if (!path) return;
    const previous = SOUND_SLOTS[slot].read(soundRef.current).audio;
    try {
      const name = await invoke<string>('import_sound_file', { path });
      persistSound(SOUND_SLOTS[slot].pickAudio(name));
      releaseAudio(previous);
    } catch (e) {
      toast.error(`选择音频失败：${e}`);
    }
  }

  function releaseAudio(name: string) {
    if (!name) return;
    const s = soundRef.current;
    const stillUsed = [s.startAudio, s.endAudio, s.toggleStartAudio, s.toggleEndAudio];
    if (stillUsed.includes(name)) return;
    const url = audioUrlCache.current.get(name);
    if (url) {
      URL.revokeObjectURL(url);
      audioUrlCache.current.delete(name);
    }
    invoke('delete_sound_file', { name }).catch(() => {});
  }

  function persistSound(patch: Partial<SoundSettings>) {
    // 副作用（写盘）放在 updater 外，避免 StrictMode 双调用导致重复写入。
    const next = { ...soundRef.current, ...patch };
    soundRef.current = next;
    setSound(next);
    settingsStore
      .set(SOUND_KEY, next)
      .then(() => settingsStore.save())
      .catch(() => {});
  }

  // 主题变更：立即应用（主色 / 亮暗）、持久化，并广播给其他窗口（浮窗）同步。
  function persistTheme(patch: Partial<ThemeSettings>) {
    const next = { ...theme, ...patch };
    setTheme(next);
    if (patch.color !== undefined) applyThemeColor(next.color);
    if (patch.mode !== undefined) {
      themeModeRef.current = next.mode;
      applyThemeMode(next.mode);
    }
    settingsStore
      .set(THEME_KEY, next)
      .then(() => settingsStore.save())
      .catch(() => {});
    emit('theme-changed', next).catch(() => {});
  }

  // 从下拉菜单设置明暗模式 / 主题色：应用+持久化并收起菜单。
  function pickThemeMode(mode: ThemeMode) {
    setMenuOpen(false);
    persistTheme({ mode });
  }

  function pickThemeColor(color: string) {
    setMenuOpen(false);
    persistTheme({ color });
  }

  // ── 三个启动开关：两两互斥，且都取「置值」而不是「取反」──
  // 取反 + 可能过期的 React 状态，在连点里会把刚关掉的那个又打开。置值最坏是重复设同一个值。
  // 互斥的第二步各自 try：它失败不能报成第一步失败，那时第一步其实已经生效了。

  /** 写 settings.json 并回写本地状态；失败时回滚并抛出，由调用方决定要不要继续互斥处理。 */
  async function persistAutoEnableOnStart(next: boolean) {
    const previous = autoEnableOnStart;
    setAutoEnableOnStart(next);
    try {
      await settingsStore.set(AUTO_ENABLE_ON_START_KEY, next);
      await settingsStore.save();
    } catch (e) {
      setAutoEnableOnStart(previous);
      throw e;
    }
  }

  /** 返回落定后的真实状态：写注册表可能被策略拦下，界面要跟实际走。 */
  async function applyAutostart(next: boolean) {
    const actual = await invoke<boolean>('set_autostart', { enabled: next });
    setSysInfo((prev) => ({ ...prev, autostart_enabled: actual }));
    return actual;
  }

  async function applyRunAsAdmin(next: boolean) {
    await invoke('set_run_as_admin', { enabled: next });
    setSysInfo((prev) => ({ ...prev, run_as_admin: next }));
  }

  async function handleToggleAutostart() {
    if (togglingAutostart) return;
    setTogglingAutostart(true);
    try {
      if (!(await applyAutostart(!sysInfo.autostart_enabled))) return;
      if (autoEnableOnStart) {
        try {
          await persistAutoEnableOnStart(false);
          toast.info('已关闭「启动后自动开全局」：开机就在后台连发容易误触发');
        } catch {
          toast.warning('「启动后自动开全局」没能关掉，开机可能直接开始连发');
        }
      }
      if (sysInfo.run_as_admin) {
        try {
          await applyRunAsAdmin(false);
          toast.info('已关闭「以管理员模式启动」：开机自启拉不起需要提权的程序');
        } catch {
          toast.warning('「以管理员模式启动」没能关掉，开机自启可能拉不起应用');
        }
      }
    } catch {
      toast.error('切换开机自启失败');
    } finally {
      setTogglingAutostart(false);
    }
  }

  async function handleToggleAutoEnableOnStart(next: boolean) {
    try {
      await persistAutoEnableOnStart(next);
    } catch {
      // 设置都没存上，不能顺手把用户的开机自启关掉
      toast.warning('保存「启动后自动开全局」失败');
      return;
    }
    if (!next || !sysInfo.autostart_enabled) return;
    try {
      await applyAutostart(false);
      toast.info('已关闭「开机自启」：开机就在后台连发容易误触发');
    } catch {
      toast.error('关闭开机自启失败');
    }
  }

  async function handleToggleRunAsAdmin() {
    if (togglingRunAsAdmin) return;
    const next = !sysInfo.run_as_admin;
    setTogglingRunAsAdmin(true);
    try {
      await applyRunAsAdmin(next);
      if (!next || !sysInfo.autostart_enabled) return;
      try {
        await applyAutostart(false);
        toast.info('已关闭「开机自启」：开机自启拉不起需要提权的程序');
      } catch {
        toast.warning('开机自启没能关掉，它和管理员模式同时开着会导致开机起不来');
      }
    } catch (e) {
      toast.error(`切换管理员模式失败：${e}`);
    } finally {
      setTogglingRunAsAdmin(false);
    }
  }

  function persistCloseBehavior(v: CloseBehavior | null) {
    const previous = closeBehaviorPreference;
    setCloseBehaviorPreference(v);
    const write =
      v === null
        ? settingsStore.delete(CLOSE_BEHAVIOR_KEY)
        : settingsStore.set(CLOSE_BEHAVIOR_KEY, v);
    write
      .then(() => settingsStore.save())
      .catch(() => {
        setCloseBehaviorPreference(previous);
        toast.warning('保存关闭行为偏好失败');
      });
  }

  async function toggleGlobal() {
    if (togglingGlobal) return;
    await setGlobal(!globalEnabled);
  }

  async function setGlobal(next: boolean) {
    setTogglingGlobal(true);
    try {
      await invoke('set_global_enabled', { enabled: next });
    } catch {
      toast.error('切换全局开关失败');
    } finally {
      setTogglingGlobal(false);
    }
  }

  async function handleInstallDriver(installed = interceptionInstalled) {
    if (installed === 'pending_reboot') {
      await confirm({
        title: '请重启电脑',
        description: (
          <>
            检测到游戏模式驱动处于「待重启」状态——上次安装或卸载尚未完成，
            必须重启电脑后才能使用此驱动。
            <br />
            <br />
            {WINDOWS_RESTART_HINT}
          </>
        ),
        confirmText: '知道了',
        cancelText: null,
      });
      return;
    }
    const ok = await confirm({
      title: '安装驱动',
      description: (
        <>
          游戏模式需要安装 Interception 内核驱动。点击「安装」后将弹出 UAC
          授权窗口，授权后控制台窗口会一闪而过即为安装完成。
          <br />
          <br />
          <strong>安装完成后必须重启电脑，驱动才会生效。</strong>
          <br />
          <br />
          {WINDOWS_RESTART_HINT}
        </>
      ),
      confirmText: '安装',
    });
    if (!ok) return;
    try {
      await invoke('install_driver');
      await refreshAppStatus();
    } catch (e) {
      toast.error(`安装失败：${e}`);
      return;
    }
    await confirm({
      title: '请重启电脑',
      description: (
        <>
          驱动安装程序已启动。如系统弹出「可能未正确安装此程序」提示，请点击「已正确安装此程序」。
          <br />
          <br />
          <strong>安装完成后请重启电脑</strong>。当前已切换为通用模式，重启生效前不能启用游戏模式。
          <br />
          <br />
          {WINDOWS_RESTART_HINT}
        </>
      ),
      confirmText: '知道了',
      cancelText: null,
    });
  }

  // 后端只报连上 / 断开，状态由这里现取。断开时外设模式已被后端回退成通用模式。
  // 后端只报连上 / 断开，状态由这里现取；取不到返回 undefined。
  async function refreshAppStatus(): Promise<AppStatus | undefined> {
    try {
      const status = await invoke<AppStatus>('get_app_status');
      applyAppStatus(status);
      return status;
    } catch {
      return undefined;
    }
  }

  // 后端先把外设模式回退成通用模式、再报断开，现取的状态已是回退后的。
  async function handleHidStickDisconnected() {
    const wasHidStick = inputMode === 'hidstick';
    const status = await refreshAppStatus();
    if (wasHidStick && status?.input_mode !== 'hidstick') {
      toast.warning('外设驱动断开了，已回退到通用模式。插回后会问你要不要切回外设模式');
    }
  }

  // 外设驱动连上（含启动时本就插着）而当前不是外设模式：问一句，同意就走手动选模式的同一条路径。
  // 判断用现取的状态：事件可能紧跟在一次切换之后到，渲染里的 inputMode 还是旧值。
  // 启动没落定前不问，落定后那次自查会补上；横版下切不过去，只提示不弹确认。
  async function handleHidStickConnected() {
    if (stickPromptingRef.current || !startupModeSettled) return;
    if (!importWelcome.loaded || importWelcome.pending || showImport) {
      stickPromptDeferredRef.current = true;
      return;
    }
    const status = await refreshAppStatus();
    if (!status?.hid_stick_present || status.input_mode === 'hidstick') return;
    if (switchingMode || showAgreement) return;
    if (layout === 'horizontal') {
      toast.info('检测到外设驱动。横版键鼠图下用不了外设模式，切回竖版后可在输入模式里选它');
      return;
    }
    if (confirmOpen) {
      stickPromptDeferredRef.current = true;
      return;
    }
    const current = isInputMode(status.input_mode)
      ? INPUT_MODE_LABELS[status.input_mode]
      : '当前模式';
    stickPromptingRef.current = true;
    try {
      const ok = await confirm({
        title: '检测到外设驱动',
        description: `要切换到外设模式吗？连发改由外设驱动作为真实的 USB 键盘鼠标敲出，即插即用、不用管理员权限。当前是${current}，之后也能在输入模式里切回来。`,
        confirmText: '切换到外设模式',
        cancelText: '不用',
      });
      if (ok) await selectInputModeRef.current('hidstick');
    } finally {
      stickPromptingRef.current = false;
    }
  }

  async function selectInputMode(target: InputMode) {
    if (blocksHorizontalLayout(target) && layout === 'horizontal') {
      setModePickerOpen(false);
      toast.warning(`横版键鼠图不支持${INPUT_MODE_LABELS[target]}，请先切回竖版规则列表`);
      return;
    }
    if (switchingMode || target === inputMode) return;
    setSwitchingMode(true);
    try {
      if (target === 'interception') {
        const status = await invoke<AppStatus>('get_app_status');
        applyAppStatus(status);
        if (
          status.elevated &&
          status.interception_installed !== 'not_installed' &&
          status.interception_issue
        ) {
          await showInterceptionIssue(status.interception_issue);
          return;
        }
        if (status.interception_installed === 'not_installed') {
          await handleInstallDriver(status.interception_installed);
          return;
        }
      }

      if ((target === 'interception' || target === 'ddsimple') && !elevated) {
        setModePickerOpen(false);
        const label = INPUT_MODE_LABELS[target];
        const ok = await confirm({
          title: `以管理员模式启用${label}`,
          description: (
            <>
              {label}
              需要管理员权限才能初始化驱动通道。应用将以管理员权限重启，并自动切换到{label}。
            </>
          ),
          confirmText: '以管理员重启',
          cancelText: '取消',
        });
        if (!ok) return;
        await invoke('relaunch_as_admin', { mode: target });
        return;
      }

      // 常规切换
      await invoke('set_input_mode', { mode: target });
      const status = await invoke<AppStatus>('get_app_status');
      applyAppStatus(status);
      const actual = status.input_mode;
      if (actual === target) {
        toast.success(`已切换为${INPUT_MODE_LABELS[target]}`);
      } else if (isInputMode(actual)) {
        toast.warning(
          target === 'interception'
            ? '驱动未就绪，请重启电脑后再试'
            : target === 'hidstick'
              ? `没连上外设驱动，已停留在${INPUT_MODE_LABELS[actual]}。确认插好（主板后置口更稳），或重新插拔一下再试`
              : `切换未生效，已停留在${INPUT_MODE_LABELS[actual]}`,
        );
      }
    } catch (e) {
      await refreshAppStatus();
      toast.error(`切换失败：${e}`);
    } finally {
      setSwitchingMode(false);
    }
  }

  async function showInterceptionIssue(issue: string) {
    setModePickerOpen(false);
    await invoke('set_input_mode', { mode: 'sendinput', persist: false });
    await refreshAppStatus();
    const inspect = await confirm({
      title: '游戏模式尚未就绪',
      description: `${issue} 本次临时使用通用模式，已保留原先选择的输入模式。诊断修复中可查看检测依据并导出报告。`,
      confirmText: '打开诊断',
      cancelText: '知道了',
    });
    if (inspect) setShowRepair(true);
  }

  function pushRules(updater: (prev: BurstRule[]) => BurstRule[]) {
    if (profilePersistence.isChanging()) return;
    const next = updater(rulesRef.current);
    const ticket = captureProfileWrite();
    rulesRef.current = next;
    setRules(next);
    let writable = false;
    const operation = ticket.name
      .then(() => {
        writable = true;
        return invoke('set_rules', { rules: next });
      })
      .then(() => {
        if (profilePersistence.isCurrent(ticket)) saveProfile(ticket);
      })
      .catch(async (e) => {
        if (!profilePersistence.isCurrent(ticket)) return;
        toast.error(`保存规则失败：${e}`);
        try {
          const engineRules = await invoke<BurstRule[]>('get_rules');
          if (!profilePersistence.isCurrent(ticket)) return;
          rulesRef.current = engineRules;
          setRules(engineRules);
        } catch {
          rulesRef.current = defaultRules();
          setRules(rulesRef.current);
        }
        if (writable) saveProfile(ticket);
      });
    profilePersistence.track(operation);
  }

  function addRule(mode: BurstMode = 'hold') {
    pushRules((prev) => [...prev, newRule(mode)]);
  }

  function removeRule(id: string) {
    pushRules((prev) => prev.filter((r) => r.id !== id));
  }

  function updateRule(id: string, patch: Partial<BurstRule>) {
    pushRules((prev) => prev.map((r) => (r.id === id ? { ...r, ...patch } : r)));
  }

  function toggleAdvanced(id: string) {
    setAdvancedOpen((s) => ({ ...s, [id]: !s[id] }));
  }

  // ── 横版布局 ────────────────────────────────────────────────────────────────
  async function switchLayout(next: PanelLayout) {
    // 横版是单键模型，每个键位都是 trigger == target 的重合态。后端下发的
    // coincident_toggle 为 false 时（当前是外设模式）切换连发建不出来，故禁止切到横版。
    // 判断依据取自后端能力位而不是模式名，新增后端时不必再改这里。
    //
    // 这里刻意重新拉一次而不用缓存：切换输入模式后策略是异步重取的，在它返回之前
    // keyPolicies 还是旧模式的值，用户紧接着点布局就会被错误放行，且该状态会写进 settings。
    if (next === 'horizontal') {
      const live = await invoke<KeyPolicies>('get_key_policy').catch(() => keyPolicies);
      setKeyPolicies(live);
      if (!live.coincident_toggle) {
        toast.warning(
          `${INPUT_MODE_LABELS[inputMode]}不支持横版键鼠图，请先切换到游戏模式或通用模式`,
        );
        return;
      }
    }
    setLayout(next);
    void applyLayout(next);
    settingsStore
      .set(LAYOUT_KEY, next)
      .then(() => settingsStore.save())
      .catch(() => toast.warning('保存布局失败'));
  }

  // 单键规则（横版可表达的形态）+ 统一间隔显示值
  const hSingleRules = rules.filter((r) => keyEq(r.trigger_key, r.target_key));
  const hUnifiedInterval = hSingleRules[0]?.interval_ms ?? unifiedInterval;

  // 左键点键：无 → 切换 → 长按 → 无 轮换（高级规则键不响应）
  function handleHCycleKey(key: KeyId) {
    const onKey = rules.filter((r) => keyEq(r.trigger_key, key));
    const single = onKey.find((r) => keyEq(r.target_key, key));
    if (onKey.length > 1 || (onKey.length === 1 && !single)) return;
    if (!single) {
      pushRules((prev) => [...prev, newSingleKeyRule(key, 'toggle', hUnifiedInterval)]);
    } else if (single.mode === 'toggle') {
      void handleHSetMode(single, 'hold');
    } else {
      removeRule(single.id);
    }
  }

  // 横版单键：切换连发的停止键就是这个键本身，轮换出来的规则总是启用
  function handleHSetMode(rule: BurstRule, mode: BurstMode) {
    setRuleMode(rule, mode, {
      stop_key: mode === 'toggle' ? rule.trigger_key : null,
      enabled: true,
    });
  }

  /** 改模式保留分组：长按也能进组插队。 */
  function setRuleMode(rule: BurstRule, mode: BurstMode, extra: Partial<BurstRule> = {}) {
    // 后端不支持切换连发的重合态（外设模式）时，启动键 == 连发按键的规则改成切换是空集，
    // 写进去也会在装载时被净化掉，不如当场拒绝并说明。
    if (
      mode === 'toggle' &&
      !keyPolicies.coincident_toggle &&
      keyEq(rule.trigger_key, rule.target_key)
    ) {
      toast.warning(
        `${INPUT_MODE_LABELS[inputMode]}下切换连发的启动键不能与连发按键相同，请先在高级设置里改长按键`,
      );
      return;
    }
    pushGroupMove((prev) =>
      prev.map((r) => (r.id === rule.id ? { ...r, mode, stop_key: null, ...extra } : r)),
    );
  }

  /** 同时含长按与切换的分组。 */
  function mixedGroups(list: BurstRule[]): Set<string> {
    const modes = new Map<string, Set<BurstMode>>();
    for (const r of list) {
      if (!r.group) continue;
      const set = modes.get(r.group) ?? new Set<BurstMode>();
      set.add(r.mode);
      modes.set(r.group, set);
    }
    return new Set([...modes].filter(([, m]) => m.size > 1).map(([g]) => g));
  }

  /**
   * 会改变分组成员或成员模式的写入都走这里（拖拽、移入分组、换模式、重命名合组）：某组从
   * 非混排变成长按与切换混排时提示插队语义（D10），直到勾选「不再提示」。按组的状态变化判定
   * 而不只提示一次：用户每次造出混排组都是在依赖插队语义，没关提示就该再看到。
   * 双向都提示——长按拖进切换组、切换拖进长按组，结果一样，用户同样需要知道长按会插队。
   */
  function pushGroupMove(updater: (prev: BurstRule[]) => BurstRule[]) {
    const before = mixedGroups(rules);
    const next = updater(rules);
    pushRules(() => next);
    if ([...mixedGroups(next)].some((g) => !before.has(g))) void showHoldInGroupHint();
  }

  async function showHoldInGroupHint() {
    const dismissed = await settingsStore.get<boolean>(HOLD_IN_GROUP_HINT_KEY).catch(() => false);
    if (dismissed) return;
    let dontShowAgain = false;
    await confirm({
      title: '长按和切换在同一组',
      body: (
        <>
          <p>按住组里的长按键时，同组正在跑的规则会暂停让位；松手后它自动接着跑。</p>
          <label className="confirm-dismiss">
            <input
              type="checkbox"
              onChange={(e) => {
                dontShowAgain = e.target.checked;
              }}
            />
            不再提示
          </label>
        </>
      ),
      confirmText: '知道了',
      cancelText: null,
    });
    if (dontShowAgain) {
      settingsStore
        .set(HOLD_IN_GROUP_HINT_KEY, true)
        .then(() => settingsStore.save())
        .catch(() => {});
    }
  }

  function handleHSetEnabled(ruleId: string, enabled: boolean) {
    updateRule(ruleId, { enabled });
  }

  function handleHDeleteKey(key: KeyId) {
    pushRules((prev) => prev.filter((r) => !keyEq(r.trigger_key, key)));
  }

  // 统一间隔：批量写入所有单键规则
  function handleHSetInterval(ms: number) {
    setUnifiedInterval(ms);
    pushRules((prev) =>
      prev.map((r) => (keyEq(r.trigger_key, r.target_key) ? { ...r, interval_ms: ms } : r)),
    );
  }

  function handleHSetGroup(ruleId: string, group: string | null) {
    pushGroupMove((prev) => prev.map((r) => (r.id === ruleId ? { ...r, group } : r)));
  }

  function handleHCreateGroupWith(ruleId: string) {
    const existingGroups = new Set(rules.map((r) => r.group).filter(Boolean));
    let n = 1;
    while (existingGroups.has(`互斥组${n}`)) n++;
    updateRule(ruleId, { group: `互斥组${n}` });
  }

  function handleNewGroup() {
    const existingGroups = new Set(rules.map((r) => r.group).filter(Boolean));
    let n = 1;
    while (existingGroups.has(`互斥组${n}`)) n++;
    const name = `互斥组${n}`;
    // 待命空组靠拖入成形，筛选下不接受拖入，故回到全部。
    setFilter('all');
    setPendingGroupName(name);
    setEditingGroupName({ current: name, draft: name });
  }

  async function disbandGroup(groupName: string) {
    const ok = await confirm({
      title: '解散分组',
      description: `将「${groupName}」解散，组内规则保留但不再互斥。`,
      confirmText: '解散',
      tone: 'danger',
    });
    if (!ok) return;
    pushRules((prev) => prev.map((r) => (r.group === groupName ? { ...r, group: null } : r)));
  }

  function toggleGroupCollapse(groupName: string) {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(groupName)) next.delete(groupName);
      else next.add(groupName);
      return next;
    });
  }

  function commitRenameGroup(oldName: string, newName: string) {
    const trimmed = newName.trim();
    const name = trimmed || oldName;
    if (pendingGroupName === oldName) setPendingGroupName(name);
    // 改成已有组的名字等于合组，可能造出混排组，同样要走 D10 判定。
    if (trimmed)
      pushGroupMove((prev) => prev.map((r) => (r.group === oldName ? { ...r, group: name } : r)));
  }

  function handleDropBeforeRule(ruleId: string, targetRuleId: string) {
    pushGroupMove((prev) => {
      const dragged = prev.find((r) => r.id === ruleId);
      const target = prev.find((r) => r.id === targetRuleId);
      if (!dragged || !target) return prev;
      const without = prev.filter((r) => r.id !== ruleId);
      const targetIdx = without.findIndex((r) => r.id === targetRuleId);
      const updated = { ...dragged, group: target.group };
      without.splice(targetIdx, 0, updated);
      return without;
    });
  }

  function handleDropToGroup(ruleId: string, groupName: string) {
    pushGroupMove((prev) => {
      const dragged = prev.find((r) => r.id === ruleId);
      if (!dragged) return prev;
      const without = prev.filter((r) => r.id !== ruleId);
      const lastInGroup = without.reduce<number>(
        (acc, r, i) => (r.group === groupName ? i : acc),
        -1,
      );
      const updated = { ...dragged, group: groupName };
      if (lastInGroup === -1) return [...without, updated];
      without.splice(lastInGroup + 1, 0, updated);
      return without;
    });
  }

  function handleDropToUngrouped(ruleId: string) {
    pushRules((prev) => prev.map((r) => (r.id === ruleId ? { ...r, group: null } : r)));
  }

  function handleDropToPending(ruleId: string) {
    const name = editingGroupName?.draft.trim() || pendingGroupName;
    if (!name) return;
    setPendingGroupName(null);
    setEditingGroupName(null);
    pushGroupMove((prev) => prev.map((r) => (r.id === ruleId ? { ...r, group: name } : r)));
  }

  /** 取出拖拽源规则并复位拖拽状态。 */
  function takeDragSource(e: ReactDragEvent): string | undefined {
    const srcId = draggingIdRef.current || e.dataTransfer.getData('text/plain');
    draggingIdRef.current = undefined;
    setDraggingId(undefined);
    setDragOverInfo(undefined);
    return srcId || undefined;
  }

  function cardDragHandlers(rule: BurstRule): CardDragHandlers {
    return {
      onDragStart: (e) => {
        draggingIdRef.current = rule.id;
        e.dataTransfer.setData('text/plain', rule.id);
        e.dataTransfer.effectAllowed = 'move';
        setDraggingId(rule.id);
      },
      onDragOver: (e) => {
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = 'move';
        setDragOverInfo({ kind: 'rule', ruleId: rule.id });
      },
      onDrop: (e) => {
        e.preventDefault();
        e.stopPropagation();
        const srcId = takeDragSource(e);
        if (!srcId || srcId === rule.id) return;
        // 放到别组的卡片前等于移入那个组；筛选下组里有被隐藏的成员，不允许这样拖入。
        const dragged = rules.find((r) => r.id === srcId);
        if (filter !== 'all' && rule.group && dragged?.group !== rule.group) return;
        handleDropBeforeRule(srcId, rule.id);
      },
      onDragEnd: () => {
        draggingIdRef.current = undefined;
        setDraggingId(undefined);
        setDragOverInfo(undefined);
      },
    };
  }

  function addRuleFromList(mode: BurstMode) {
    addRule(mode);
    // 新规则落在当前筛选之外会看不见，像是没加上。
    if (filter !== 'all' && filter !== mode) setFilter('all');
  }

  /** 组头状态区的规则名：启动键与连发按键相同时只写一个键名。 */
  function ruleLabel(rule: BurstRule): string {
    return keyEq(rule.trigger_key, rule.target_key)
      ? keyLabel(rule.target_key)
      : `${keyLabel(rule.trigger_key)}→${keyLabel(rule.target_key)}`;
  }

  async function handleDelete(id: string) {
    const ok = await confirm({
      title: '删除规则',
      description: '确认删除此规则？此操作不可撤销。',
      confirmText: '删除',
      tone: 'danger',
    });
    if (ok) removeRule(id);
  }

  async function switchToProfile(path: string) {
    try {
      await changeProfile(async () => {
        const profile = await invoke<Profile>('load_profile', { path });
        applyLoadedProfile(profile);
        await refreshProfileList();
      });
    } catch (e) {
      toast.error(`切换配置失败：${e}`);
    }
  }

  async function handleCreateProfile() {
    let name = '';
    const ok = await confirm({
      title: '新建配置',
      description: '请输入新配置的名称：',
      confirmText: '创建',
      body: (
        <ProfileNameForm
          placeholder="例如：刺客 / 法师 / 测试用"
          onChange={(v) => {
            name = v;
          }}
        />
      ),
    });
    if (!ok) return;
    const trimmed = name.trim();
    if (!trimmed) {
      toast.warning('配置名不能为空');
      return;
    }
    if (trimmed === DEFAULT_PROFILE_NAME) {
      toast.warning('不能使用默认配置名');
      return;
    }
    if (profileList.some((p) => p.meta.name === trimmed)) {
      toast.warning('已存在同名配置');
      return;
    }
    // 复用 fork：会基于「当前激活配置」创建副本，名字冲突由后端 pick_unique_name 兜底
    try {
      await changeProfile(async () => {
        const res = await invoke<ForkResult>('fork_active_profile', { suggestedName: trimmed });
        applyLoadedProfile(res.profile);
        await refreshProfileList();
        toast.success(`已创建配置「${res.profile.meta.name}」`);
      });
    } catch (e) {
      toast.error(`创建失败：${e}`);
    }
  }

  async function handleRenameProfileByName(name: string) {
    if (name === DEFAULT_PROFILE_NAME) {
      toast.warning('默认配置不可重命名');
      return;
    }
    let nextName = name;
    const ok = await confirm({
      title: '重命名配置',
      description: `配置「${name}」的新名称：`,
      confirmText: '重命名',
      body: (
        <ProfileNameForm
          defaultValue={name}
          onChange={(v) => {
            nextName = v;
          }}
        />
      ),
    });
    if (!ok) return;
    const trimmed = nextName.trim();
    if (!trimmed || trimmed === name) return;
    if (trimmed === DEFAULT_PROFILE_NAME) {
      toast.warning('不能使用默认配置名');
      return;
    }
    if (profileList.some((p) => p.meta.name === trimmed && p.meta.name !== name)) {
      toast.warning('已存在同名配置');
      return;
    }
    try {
      await changeProfile(async () => {
        await invoke<string>('rename_profile', {
          oldName: name,
          newName: trimmed,
        });
        if (name === profileNameRef.current) {
          profileNameRef.current = trimmed;
          setProfileName(trimmed);
        }
        await refreshProfileList();
        toast.success(`已重命名为「${trimmed}」`);
      });
    } catch (e) {
      toast.error(`重命名失败：${e}`);
    }
  }

  async function handleSignatureChange(signature: string) {
    try {
      await changeProfile(async () => {
        await ensureWritableProfile();
        if (profileNameRef.current === DEFAULT_PROFILE_NAME) return;
        await invoke<string>('set_profile_signature', {
          name: profileNameRef.current,
          signature,
        });
        await refreshProfileList();
      });
    } catch (e) {
      toast.error(`保存署名失败：${e}`);
    }
  }

  async function handleDeleteProfileByName(name: string) {
    if (name === DEFAULT_PROFILE_NAME) {
      toast.warning('默认配置不可删除');
      return;
    }
    const ok = await confirm({
      title: '删除配置',
      description:
        name === profileName
          ? `确定删除当前配置「${name}」？删除后将自动切换为默认配置。`
          : `确定删除配置「${name}」？`,
      confirmText: '删除',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await changeProfile(async () => {
        const fallback = await invoke<Profile | null>('delete_profile', { name });
        if (fallback) applyLoadedProfile(fallback);
        await refreshProfileList();
        toast.success('配置已删除');
      });
    } catch (e) {
      toast.error(`删除失败：${e}`);
    }
  }

  async function handleExportProfileByName(name: string) {
    const displayName = name === DEFAULT_PROFILE_NAME ? '默认配置' : name;
    const dest = await saveFileDialog({
      defaultPath: `${displayName}.qzh`,
      filters: [{ name: '配置文件', extensions: ['qzh'] }],
    });
    if (!dest) return;
    try {
      await profilePersistence.flush();
      await invoke<string>('export_profile', { name, destPath: dest });
      toast.success(`已导出「${displayName}」`);
    } catch (e) {
      toast.error(`导出失败：${e}`);
    }
  }

  async function handleClose() {
    try {
      await profilePersistence.flush();
    } catch (e) {
      toast.error(`关闭前保存失败：${e}`);
      return;
    }
    settingsStore
      .get<CloseBehavior>(CLOSE_BEHAVIOR_KEY)
      .then((remembered) => {
        if (remembered === 'exit') invoke('exit_app').catch(() => {});
        else if (remembered === 'minimize') invoke('minimize_to_float').catch(() => {});
        else void askCloseBehavior();
      })
      .catch(() => {
        toast.error('读取关闭行为偏好失败');
        void askCloseBehavior();
      });
  }

  async function askCloseBehavior() {
    const result: { choice: CloseBehavior; remember: boolean } = {
      choice: 'minimize',
      remember: false,
    };
    const ok = await confirm({
      title: '关闭窗口',
      description: '关闭按钮的行为：',
      confirmText: '确定',
      body: (
        <CloseBehaviorForm
          defaultChoice="minimize"
          onChange={(c, r) => {
            result.choice = c;
            result.remember = r;
          }}
        />
      ),
    });
    if (!ok) return;
    if (result.remember) persistCloseBehavior(result.choice);
    if (result.choice === 'exit') invoke('exit_app').catch(() => {});
    else invoke('minimize_to_float').catch(() => {});
  }

  function handleShowSettings(tab: SettingsTab = 'general') {
    setMenuOpen(false);
    setProfileMenuOpen(false);
    setSettingsTab(tab);
    setShowSettings(true);
  }

  function closeMenus() {
    setMenuOpen(false);
    setModePickerOpen(false);
    setProfileMenuOpen(false);
  }

  const tourSnapshot: TourSnapshot = useMemo(
    () => ({
      rules,
      globalEnabled,
      inputMode,
      layout,
      filter,
      collapsedGroups: [...collapsedGroups].sort(),
      keyCaptureSeq,
      settingsOpen: showSettings,
      settingsTab,
      coincidentToggle: keyPolicies.coincident_toggle,
      runningRuleIds: [...activeRuleIds].filter((id) => !pausedRuleIds.has(id)).sort(),
      pausedRuleIds: [...pausedRuleIds].sort(),
    }),
    [
      rules,
      globalEnabled,
      inputMode,
      layout,
      filter,
      collapsedGroups,
      keyCaptureSeq,
      showSettings,
      settingsTab,
      keyPolicies.coincident_toggle,
      activeRuleIds,
      pausedRuleIds,
    ],
  );

  const tourHost: TourHost = useMemo(
    () => ({
      snapshot: tourSnapshot,
      setFilter,
      setLayout: switchLayout,
      openSettings: handleShowSettings,
      closeSettings: () => setShowSettings(false),
      openJx3: () => {
        closeMenus();
        setShowJx3(true);
      },
      closeJx3: () => setShowJx3(false),
      closeMenus,
      createSampleGroup,
      deleteSampleGroup,
    }),
    // 动作都是组件内的普通函数、每次渲染新建，纳入依赖等于每帧换一个 host；
    // 只让快照变化驱动重建
    [tourSnapshot],
  );

  /**
   * 互斥组教程的示例组：两条切换 + 一条长按，全部停用。键位从候选里挑没被占用的——撞上用户
   * 自己的规则会同时触发、冲突提醒变红，教程就讲不清了。走 pushRules 不走 pushGroupMove：
   * 教程本身就在讲插队，不再弹 D10 提示。
   */
  function createSampleGroup(): boolean {
    if (sampleRules({ rules })) return true;
    // 残留的部分示例规则（删了一条、拖出组）当脏数据清掉再重建，否则建组判定与教程判定
    // 口径不一，按钮成了空操作、第 2 步永远等不到完成。挑键与写入都基于 updater 的 prev
    // 派生，不拿渲染时的快照写盘；这里先挑一次只为决定要不要提示。
    if (!pickSampleKeys(rules.filter((r) => !isSampleRule(r)))) {
      toast.warning('键位都被占用了，请先在规则或热键里腾出几个数字键和字母键，或跳过这一步');
      return false;
    }
    pushRules((prev) => {
      const kept = prev.filter((r) => !isSampleRule(r));
      const keys = pickSampleKeys(kept);
      if (!keys) return prev;
      const sample = (id: string, mode: BurstMode, trigger: number, target: number): BurstRule => ({
        id,
        enabled: false,
        trigger_key: keyboardKey(trigger),
        target_key: keyboardKey(target),
        mode,
        stop_key: null,
        interval_ms: 50,
        group: SAMPLE_GROUP,
      });
      return [
        ...kept,
        sample(SAMPLE_IDS.a, 'toggle', keys.pair[0], keys.targets[0]),
        sample(SAMPLE_IDS.b, 'toggle', keys.pair[1], keys.targets[1]),
        sample(SAMPLE_IDS.hold, 'hold', keys.hold, keys.targets[2]),
      ];
    });
    setFilter('all');
    return true;
  }

  /**
   * 示例组键位：从候选里挑没被占用的。撞上用户规则会同时触发、冲突提醒变红，教程就讲不清了。
   * 占用 = 规则的启动 / 连发 / 停止键 ∪ 三个全局热键；挑不齐返回 undefined。
   */
  function pickSampleKeys(
    list: BurstRule[],
  ): { pair: [number, number]; hold: number; targets: number[] } | undefined {
    const used = new Set<string>();
    const mark = (k: KeyId | null) => k && used.add(keyToken(k));
    for (const r of list) {
      mark(r.trigger_key);
      mark(r.target_key);
      mark(r.stop_key);
    }
    mark(hotkeys.global_toggle);
    mark(hotkeys.global_stop);
    mark(hotkeys.panel_toggle);
    const free = (vk: number) => !used.has(keyToken(keyboardKey(vk)));
    const pairs: [number, number][] = [
      [0x31, 0x32],
      [0x33, 0x34],
      [0x35, 0x36],
      [0x37, 0x38],
      [0x39, 0x30],
      [0x5a, 0x58],
    ];
    const holds = [0x56, 0x42, 0x4e, 0x4d, 0x48, 0x4a]; // V B N M H J
    const targetCandidates = [0x51, 0x45, 0x52, 0x54, 0x59, 0x55, 0x49, 0x4f, 0x50]; // Q E R T Y U I O P
    const pair = pairs.find(([a, b]) => free(a) && free(b));
    const hold = holds.find(free);
    const targets = targetCandidates.filter(free).slice(0, 3);
    if (!pair || hold === undefined || targets.length < 3) return undefined;
    return { pair, hold, targets };
  }

  function deleteSampleGroup() {
    pushRules((prev) => prev.filter((r) => !isSampleRule(r)));
  }

  function logTourEvent(id: string, event: string) {
    invoke('log_from_frontend', { level: 'info', message: `tour ${id} ${event}` }).catch(() => {});
  }

  function startTour(tour: TourDef) {
    closeMenus();
    setShowTourCatalog(false);
    logTourEvent(tour.id, 'started');
    setActiveTour(tour);
  }

  function handleTourExit(result: TourExitResult) {
    const tour = activeTour;
    if (tour) {
      logTourEvent(tour.id, result);
      if (result === 'completed') {
        tourProgress.markCompleted(tour.id).catch(() => toast.warning('保存教程进度失败'));
      }
    }
    tourProgress.markIntroSeen().catch(() => {});
    setActiveTour(undefined);
  }

  function handleCheckUpdate() {
    setMenuOpen(false);
    updateDownloadFailedRef.current = false;
    invoke('check_update').catch(() => {
      setUpdateProgress(null);
      if (!updateDownloadFailedRef.current) {
        toast.warning('检查更新失败，请检查网络连接后重试');
      }
    });
  }

  function handleToggleAutoUpdate(next: boolean) {
    setAutoUpdate(next);
    settingsStore
      .set(AUTO_UPDATE_KEY, next)
      .then(() => settingsStore.save())
      .catch(() => toast.warning('保存自动更新设置失败'));
  }

  // 标题栏提示：已下好就直接开弹窗，只是查到新版本则走一次手动检查（必定下载）
  function handleUpdateIndicator() {
    if (updateReady) {
      setUpdateNotice({ info: updateReady, mode: 'ready' });
      return;
    }
    handleCheckUpdate();
  }

  async function handleApplyUpdate() {
    setApplyingUpdate(true);
    try {
      await invoke('apply_pending_update');
    } catch (e) {
      setApplyingUpdate(false);
      toast.warning(`安装更新失败：${e}`);
    }
  }

  function openSupport() {
    invoke('open_link', { target: 'support' }).catch((e) => toast.error(`打开支持&帮助失败：${e}`));
  }

  // 先复制邮箱再打开 QQ 邮箱网页：网页版没有能预填收件人的写信链接，只能让用户粘贴
  async function writeMail() {
    let copied = true;
    try {
      await navigator.clipboard.writeText(AUTHOR_EMAIL);
    } catch {
      copied = false;
    }
    try {
      await invoke('open_link', { target: 'mail' });
    } catch {
      toast.warning(`打不开浏览器，请手动发邮件到 ${AUTHOR_EMAIL}`);
      return;
    }
    toast.success(
      copied
        ? '已复制邮箱，在打开的 QQ 邮箱里点「写信」，粘贴到收件人'
        : `在打开的 QQ 邮箱里点「写信」，收件人填 ${AUTHOR_EMAIL}`,
    );
  }

  function handleShowAbout() {
    setMenuOpen(false);
    setShowAbout(true);
  }

  function handleShowRepair() {
    setMenuOpen(false);
    setShowRepair(true);
  }

  async function handleUninstallDriver() {
    const ok = await confirm({
      title: '卸载驱动',
      description: (
        <>
          将卸载 Interception 内核驱动。卸载后游戏模式将不可用，应用会切回通用模式。
          <br />
          <br />
          <strong>卸载完成后必须重启电脑才能彻底生效。</strong>
          <br />
          <br />
          {WINDOWS_RESTART_HINT}
        </>
      ),
      confirmText: '卸载',
      cancelText: '取消',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await invoke('uninstall_driver');
      await confirm({
        title: '卸载完成',
        description: (
          <>
            驱动已卸载。如系统弹出「可能未正确安装此程序」提示，请点击「已正确安装此程序」。
            <br />
            <br />
            <strong>请重启电脑使卸载彻底生效。</strong>
            <br />
            <br />
            {WINDOWS_RESTART_HINT}
          </>
        ),
        confirmText: '知道了',
        cancelText: null,
      });
    } catch (e) {
      toast.error(`卸载失败：${e}`);
    }
  }

  async function handleUninstallDdHid() {
    const ok = await confirm({
      title: '卸载 DDHID 驱动',
      description: (
        <>
          DDHID 驱动不稳定、可能导致蓝屏，本应用已不再使用它。这里会清理你机器上残留的 DDHID
          虚拟驱动。
          <br />
          <br />
          卸载流程会调用 PnP 标准接口处理，建议卸载完成后重启电脑。
          <br />
          <br />
          {WINDOWS_RESTART_HINT}
        </>
      ),
      confirmText: '卸载',
      cancelText: '取消',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      const r = await invoke<{ message: string; pending_reboot: boolean }>(
        'uninstall_dd_hid_driver',
      );
      if (r.pending_reboot) {
        toast.warning(`${r.message}\n${WINDOWS_RESTART_HINT}`, 12000);
      } else {
        toast.success(`${r.message || 'DDHID 驱动已卸载'}\n${WINDOWS_RESTART_HINT}`, 12000);
      }
    } catch (e) {
      toast.error(`卸载失败：${e}`);
    }
  }

  function handleAgreed() {
    setShowAgreement(false);
    // 启动收尾那一问还没问到时交给它，免得声音设置没读到就开、按默认值播报
    if (
      startupAutoEnableCheckedRef.current &&
      importWelcome.loaded &&
      !importWelcome.pending &&
      !showImport
    )
      void applyStartupAutoEnable();
  }

  return (
    <div
      className={`panel ${layout === 'horizontal' ? 'panel--h' : 'panel--v'}${globalEnabled ? ' on' : ' off'}`}
      data-signature={activeSignature}
      style={{ ['--panel-bg' as string]: `url(${bgUrl})` }}
    >
      {globalEnabled && <div className="global-active-frame" aria-hidden="true" />}
      <header className="panel-header" onMouseDown={handleHeaderMouseDown}>
        <img className="header-icon" src={iconUrl} alt="" draggable={false} />
        <h1>
          {APP_NAME}
          {hotkeys.panel_toggle && (
            <Kbd title="面板显隐快捷键">{keyLabel(hotkeys.panel_toggle)}</Kbd>
          )}
        </h1>
        <div className="window-controls">
          <button
            className="win-btn"
            data-tour="layout-toggle"
            onClick={() => void switchLayout(layout === 'vertical' ? 'horizontal' : 'vertical')}
            // 用 aria-disabled 而非 disabled：disabled 的按钮不触发 onClick，
            // switchLayout 里的原因提示就永远跑不到，点下去毫无反应。
            aria-disabled={!keyPolicies.coincident_toggle && layout === 'vertical'}
            aria-label="切换布局"
            title={
              !keyPolicies.coincident_toggle && layout === 'vertical'
                ? `${INPUT_MODE_LABELS[inputMode]}不支持横版键鼠图`
                : layout === 'vertical'
                  ? '切换到横版键鼠图'
                  : '切换到竖版规则列表'
            }
          >
            {layout === 'vertical' ? (
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
                <rect x="1.5" y="3" width="13" height="4" rx="1" stroke="currentColor" />
                <rect x="1.5" y="9" width="13" height="4" rx="1" stroke="currentColor" />
              </svg>
            ) : (
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
                <rect x="3" y="1.5" width="4" height="13" rx="1" stroke="currentColor" />
                <rect x="9" y="1.5" width="4" height="13" rx="1" stroke="currentColor" />
              </svg>
            )}
          </button>
          {(updateReady || updateAvailable) && (
            <button
              className="win-btn win-btn--update"
              onClick={handleUpdateIndicator}
              aria-label="有可用更新"
              title={
                updateReady
                  ? `新版本 v${updateReady.version} 已下载，点击查看并安装`
                  : `发现新版本 v${updateAvailable?.version}，点击下载更新`
              }
            >
              <UpdateIcon size={14} />
              <span className="win-btn__dot" aria-hidden="true" />
            </button>
          )}
          <button
            ref={menuBtnRef}
            className="win-btn menu-btn"
            data-tour="menu"
            onClick={() => setMenuOpen((v) => !v)}
            aria-label="菜单"
          >
            <MenuIcon size={14} />
          </button>
          <button
            className="win-btn"
            data-tour="minimize"
            onClick={() => invoke('minimize_to_float').catch(() => {})}
            aria-label="最小化到浮窗"
          >
            <MinimizeIcon size={14} />
          </button>
          <button
            className="win-btn"
            onClick={() => void toggleMaximize()}
            aria-label={maximized ? '还原' : '最大化'}
            title={maximized ? '还原' : '最大化'}
          >
            {maximized ? (
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
                <rect x="2.5" y="5.5" width="8" height="8" rx="1" stroke="currentColor" />
                <path
                  d="M5.5 5.5V3.5a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-2"
                  stroke="currentColor"
                />
              </svg>
            ) : (
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
                <rect x="2.5" y="2.5" width="11" height="11" rx="1" stroke="currentColor" />
              </svg>
            )}
          </button>
          <button className="win-btn close" onClick={handleClose} aria-label="关闭">
            <CloseIcon size={14} />
          </button>
        </div>
      </header>
      {updateProgress && <UpdateProgressBar progress={updateProgress} />}

      <section className="rules-section">
        {layout === 'vertical' && (
          <div data-tour="filter">
            <Tabs
              tabs={[
                { id: 'all' as RuleFilter, label: '全部', badge: `${rules.length}` },
                ...(['hold', 'toggle'] as BurstMode[]).map((mode) => {
                  const inMode = rules.filter((r) => r.mode === mode);
                  const enabled = inMode.filter((r) => r.enabled).length;
                  return {
                    id: mode as RuleFilter,
                    label: mode === 'hold' ? '长按' : '切换',
                    badge: `${enabled}/${inMode.length}`,
                  };
                }),
              ]}
              active={filter}
              grow
              onChange={setFilter}
            />
          </div>
        )}

        {layout === 'horizontal' && (
          <HorizontalLayout
            policy={keyPolicies.trigger_target}
            rules={rules}
            activeRuleIds={activeRuleIds}
            pausedRuleIds={pausedRuleIds}
            conflicts={conflicts}
            interval={hUnifiedInterval}
            intervalMin={MIN_INTERVAL_MS}
            intervalMax={MAX_INTERVAL_MS}
            onIntervalChange={handleHSetInterval}
            onCycleKey={handleHCycleKey}
            onSetEnabled={handleHSetEnabled}
            onSetMode={handleHSetMode}
            onSetGroup={handleHSetGroup}
            onCreateGroupWith={handleHCreateGroupWith}
            onDeleteRule={removeRule}
            onDeleteKey={handleHDeleteKey}
          />
        )}

        {layout === 'vertical' &&
          (() => {
            const matches = (r: BurstRule) => filter === 'all' || r.mode === filter;
            const visible = rules.filter(matches);
            const latestId = lastRule(tourSnapshot)?.id;
            // 组容器按第一个成员出现的位置整体渲染，与拖拽排序（数组顺序）同一口径。
            const groupNames = [
              ...new Set(rules.filter((r) => r.group).map((r) => r.group as string)),
            ];
            const latestGroup = groupNames.includes(SAMPLE_GROUP)
              ? SAMPLE_GROUP
              : groupNames[groupNames.length - 1];
            const draggingRule = draggingId ? rules.find((r) => r.id === draggingId) : undefined;
            const canGroupDrop = filter === 'all';
            const groupList = groupNames.filter((g) => g !== pendingGroupName);

            const renderCard = (rule: BurstRule) => (
              <RuleCard
                key={rule.id}
                rule={rule}
                isActive={activeRuleIds.has(rule.id)}
                isPaused={pausedRuleIds.has(rule.id)}
                isLatest={rule.id === latestId}
                showAdvanced={!!advancedOpen[rule.id]}
                isDragging={draggingId === rule.id}
                isDragTarget={
                  dragOverInfo?.kind === 'rule' &&
                  dragOverInfo.ruleId === rule.id &&
                  draggingId !== rule.id
                }
                conflict={severityForRule(conflicts, rule.id)}
                policies={keyPolicies}
                intervalMin={MIN_INTERVAL_MS}
                intervalMax={MAX_INTERVAL_MS}
                groups={groupList}
                drag={cardDragHandlers(rule)}
                onPatch={(patch) => updateRule(rule.id, patch)}
                onSetMode={(mode) => setRuleMode(rule, mode)}
                onToggleAdvanced={() => toggleAdvanced(rule.id)}
                onDelete={() => void handleDelete(rule.id)}
                onMoveToGroup={(g) => handleDropToGroup(rule.id, g)}
                onNewGroupWith={() => handleHCreateGroupWith(rule.id)}
                onRemoveFromGroup={() => handleDropToUngrouped(rule.id)}
                onKeyReject={notifyRuleKeyReject}
                onTargetCaptured={() => setKeyCaptureSeq((n) => n + 1)}
              />
            );

            const renderGroup = (name: string) => {
              const members = rules.filter((r) => r.group === name);
              const shown = members.filter(matches);
              return (
                <RuleGroup
                  key={`group:${name}`}
                  name={name}
                  isLatest={name === latestGroup}
                  collapsed={collapsedGroups.has(name)}
                  draft={editingGroupName?.current === name ? editingGroupName.draft : undefined}
                  dragActive={dragOverInfo?.kind === 'group' && dragOverInfo.name === name}
                  acceptDrop={canGroupDrop}
                  hasHold={members.some((r) => r.mode === 'hold')}
                  running={members
                    .filter((r) => activeRuleIds.has(r.id) && !pausedRuleIds.has(r.id))
                    .map((r) => ({ id: r.id, label: ruleLabel(r) }))}
                  paused={members
                    .filter((r) => pausedRuleIds.has(r.id))
                    .map((r) => ({ id: r.id, label: ruleLabel(r) }))}
                  hiddenCount={members.length - shown.length}
                  onToggleCollapse={() => toggleGroupCollapse(name)}
                  onStartRename={() => setEditingGroupName({ current: name, draft: name })}
                  onDraftChange={(draft) => setEditingGroupName((g) => g && { ...g, draft })}
                  onCommitRename={() => {
                    if (editingGroupName) {
                      commitRenameGroup(editingGroupName.current, editingGroupName.draft);
                      setEditingGroupName(null);
                    }
                  }}
                  onCancelRename={() => setEditingGroupName(null)}
                  onDisband={() => void disbandGroup(name)}
                  onDragOver={() => setDragOverInfo({ kind: 'group', name })}
                  onDrop={(e) => {
                    const srcId = takeDragSource(e);
                    if (srcId) handleDropToGroup(srcId, name);
                  }}
                >
                  {shown.map(renderCard)}
                </RuleGroup>
              );
            };

            const items: ReactNode[] = [];
            const seenGroups = new Set<string>();
            for (const rule of rules) {
              if (!rule.group) {
                if (matches(rule)) items.push(renderCard(rule));
              } else if (!seenGroups.has(rule.group)) {
                seenGroups.add(rule.group);
                items.push(renderGroup(rule.group));
              }
            }

            return (
              <div className="rule-group" key="list">
                <div className="rules-list">
                  {rules.length === 0 && !pendingGroupName && <p className="empty">暂无连发规则</p>}
                  {rules.length > 0 && visible.length === 0 && groupNames.length === 0 && (
                    <p className="empty">没有{filter === 'hold' ? '长按' : '切换'}连发规则</p>
                  )}

                  {items}

                  {/* 拖动有分组规则时显示"移出分组"区域 */}
                  {draggingId && draggingRule?.group && (
                    <div
                      className={`ungrouped-escape-zone${dragOverInfo?.kind === 'ungrouped' ? ' drag-active' : ''}`}
                      onDragOver={(e) => {
                        e.preventDefault();
                        setDragOverInfo({ kind: 'ungrouped' });
                      }}
                      onDrop={(e) => {
                        e.preventDefault();
                        const srcId = takeDragSource(e);
                        if (srcId) handleDropToUngrouped(srcId);
                      }}
                    >
                      移出分组
                    </div>
                  )}

                  {/* 待命空分组（新建后等待拖入或改名） */}
                  {pendingGroupName &&
                    (() => {
                      const isPendingEditing = editingGroupName?.current === pendingGroupName;
                      const isPendingDragOver =
                        dragOverInfo?.kind === 'group' && dragOverInfo.name === pendingGroupName;
                      const pendingDrop = (e: ReactDragEvent<HTMLDivElement>) => {
                        e.preventDefault();
                        e.stopPropagation();
                        const srcId = takeDragSource(e);
                        if (srcId) handleDropToPending(srcId);
                      };
                      const pendingDragOver = (e: ReactDragEvent<HTMLDivElement>) => {
                        e.preventDefault();
                        e.stopPropagation();
                        setDragOverInfo({ kind: 'group', name: pendingGroupName });
                      };
                      return (
                        <div
                          key="__pending__"
                          className={`rule-group-container${isPendingDragOver ? ' drag-active' : ''}`}
                          onDragOver={pendingDragOver}
                          onDrop={pendingDrop}
                        >
                          <div className="rule-group-header">
                            {isPendingEditing ? (
                              <input
                                className="group-name-edit"
                                autoFocus
                                value={editingGroupName.draft}
                                onChange={(e) =>
                                  setEditingGroupName({
                                    current: pendingGroupName,
                                    draft: e.target.value,
                                  })
                                }
                                onBlur={() => {
                                  const name =
                                    (editingGroupName?.draft ?? '').trim() || pendingGroupName;
                                  setPendingGroupName(name);
                                  setEditingGroupName(null);
                                }}
                                onKeyDown={(e) => {
                                  if (e.key === 'Enter') {
                                    const name =
                                      (editingGroupName?.draft ?? '').trim() || pendingGroupName;
                                    setPendingGroupName(name);
                                    setEditingGroupName(null);
                                  } else if (e.key === 'Escape') {
                                    setPendingGroupName(null);
                                    setEditingGroupName(null);
                                  }
                                }}
                                onDragOver={(e) => e.preventDefault()}
                                onDrop={(e) => e.preventDefault()}
                              />
                            ) : (
                              <span className="group-name-text">{pendingGroupName}</span>
                            )}
                            {!isPendingEditing && (
                              <button
                                className="group-edit-btn"
                                onClick={() =>
                                  setEditingGroupName({
                                    current: pendingGroupName,
                                    draft: pendingGroupName,
                                  })
                                }
                                title="重命名"
                              >
                                <EditIcon size={12} />
                              </button>
                            )}
                            <button
                              className="disband-btn"
                              onClick={() => {
                                setPendingGroupName(null);
                                setEditingGroupName(null);
                              }}
                              title="取消"
                            >
                              取消
                            </button>
                          </div>
                          <div className="group-body">
                            <div
                              className={`group-drop-zone${isPendingDragOver ? ' drag-active' : ''}`}
                            >
                              将规则拖入此分组
                            </div>
                          </div>
                        </div>
                      );
                    })()}
                </div>

                <div className="rules-bottom-actions" data-tour="add-buttons">
                  <Button
                    className="add-btn"
                    variant="dashed"
                    tone="primary"
                    data-tour="add-hold"
                    onClick={() => addRuleFromList('hold')}
                  >
                    + 长按连发
                  </Button>
                  <Button
                    className="add-btn"
                    variant="dashed"
                    tone="primary"
                    data-tour="add-toggle"
                    onClick={() => addRuleFromList('toggle')}
                  >
                    + 切换连发
                  </Button>
                  <Button
                    className="add-btn"
                    variant="dashed"
                    tone="neutral"
                    data-tour="add-group"
                    onClick={handleNewGroup}
                  >
                    + 新建分组
                  </Button>
                </div>
              </div>
            );
          })()}
      </section>

      <footer className="panel-footer">
        <Button
          ref={profileBtnRef}
          variant="outline"
          tone="neutral"
          size="sm"
          appendIcon={<ChevronIcon size={10} />}
          data-tour="profile"
          onClick={() => setProfileMenuOpen((v) => !v)}
          title={isDefaultProfile ? '默认配置（修改后将自动新建）' : `当前配置：${profileName}`}
        >
          {isDefaultProfile ? '默认配置' : profileName}
        </Button>
        <div className="footer-controls">
          <div className="footer-control">
            <Button
              ref={modeBtnRef}
              variant="outline"
              tone={inputMode !== 'sendinput' ? 'primary' : 'neutral'}
              size="sm"
              loading={switchingMode}
              appendIcon={<ChevronIcon size={10} />}
              data-tour="input-mode"
              onClick={() => setModePickerOpen((v) => !v)}
              title="点击选择输入模式"
            >
              {INPUT_MODE_LABELS[inputMode]}
              {inputMode === 'interception' ? ' ★' : ''}
            </Button>
          </div>
          <div className="footer-control">
            <Button
              variant="solid"
              tone={globalEnabled ? 'primary' : 'neutral'}
              size="sm"
              loading={togglingGlobal}
              kbd={(() => {
                const k = globalEnabled
                  ? (hotkeys.global_stop ?? hotkeys.global_toggle)
                  : hotkeys.global_toggle;
                return k ? keyLabel(k) : undefined;
              })()}
              data-tour="global"
              onClick={toggleGlobal}
            >
              {globalEnabled ? '全局已启用' : '全局已禁用'}
            </Button>
          </div>
        </div>
      </footer>

      <ContextMenu
        open={profileMenuOpen}
        onClose={() => setProfileMenuOpen(false)}
        target={profileBtnRef}
        location="bottom-left"
        items={buildProfileMenu({
          profiles: profileList,
          activeName: profileName,
          onSwitch: switchToProfile,
          onCreate: handleCreateProfile,
          onManage: () => handleShowSettings('profiles'),
        })}
      />

      <ContextMenu
        open={modePickerOpen}
        onClose={() => setModePickerOpen(false)}
        target={modeBtnRef}
        location="bottom-left"
        items={(['interception', 'sendinput', 'ddsimple', 'hidstick'] as InputMode[])
          .filter((m) => m !== 'hidstick' || hidStickPresent)
          .map((m) => ({
            label: `${INPUT_MODE_LABELS[m]}${m === 'interception' ? ' ★' : ''}`,
            subtitle:
              m === 'sendinput'
                ? '普通软件与网页 · 大部分游戏无效'
                : m === 'interception'
                  ? interceptionIssue && interceptionInstalled !== 'not_installed'
                    ? '驱动未就绪 · 点击查看原因'
                    : interceptionInstalled === 'installed'
                      ? elevated
                        ? '推荐 · 管理员已就绪'
                        : '推荐 · 需要管理员'
                      : interceptionInstalled === 'pending_reboot'
                        ? '推荐 · 驱动待重启生效'
                        : '推荐 · 点击安装驱动'
                  : m === 'hidstick'
                    ? hidStickPresent
                      ? '外设 · 已插入'
                      : '外设 · 需插入外设驱动'
                    : elevated
                      ? '备用 · 内置DD驱动，无需重启'
                      : '备用 · 需要管理员',
            active: inputMode === m,
            onClick: () => void selectInputMode(m),
          }))}
      />

      <ContextMenu
        open={menuOpen}
        onClose={() => setMenuOpen(false)}
        target={menuBtnRef}
        items={[
          { label: '设置', onClick: handleShowSettings },
          {
            label: '主题颜色',
            children: [
              {
                label: '明亮',
                active: theme.mode === 'light',
                onClick: () => pickThemeMode('light'),
              },
              {
                label: '黑暗',
                active: theme.mode === 'dark',
                onClick: () => pickThemeMode('dark'),
              },
              {
                label: '跟随系统',
                active: theme.mode === 'system',
                onClick: () => pickThemeMode('system'),
              },
              { type: 'divider' },
              ...SECT_PRESETS.map((p) => ({
                label: p.name,
                active: theme.color.toLowerCase() === p.color.toLowerCase(),
                prependIcon: (
                  <span
                    aria-hidden="true"
                    style={{
                      width: 12,
                      height: 12,
                      borderRadius: 3,
                      background: p.color,
                      boxShadow: 'inset 0 0 0 1px rgba(0, 0, 0, 0.15)',
                    }}
                  />
                ),
                onClick: () => pickThemeColor(p.color),
              })),
            ],
          },
          {
            label: '剑网三专栏',
            onClick: () => {
              setMenuOpen(false);
              setShowJx3(true);
            },
          },
          { type: 'divider' },
          {
            label: '新手教程',
            onClick: () => {
              setMenuOpen(false);
              setShowTourCatalog(true);
            },
          },
          {
            label: '导入配置',
            onClick: () => {
              setMenuOpen(false);
              setShowImport(true);
            },
          },
          { label: '检查更新', onClick: handleCheckUpdate },
          {
            // 固定看当前运行版本做了什么，来源是随包内联的 CHANGELOG；
            // 新版本的公告只在下载完成的弹窗里出现，两者不能混
            label: '更新公告',
            onClick: () => {
              setMenuOpen(false);
              setUpdateNotice({
                info: { version: appVersion, notes: changelogSection(appVersion) },
                mode: 'current',
              });
            },
          },
          {
            label: '用户协议',
            onClick: () => {
              setMenuOpen(false);
              setShowAgreement(true);
            },
          },
          { type: 'divider' },
          { label: '诊断修复', onClick: handleShowRepair },
          {
            label: '支持&帮助',
            onClick: () => {
              setMenuOpen(false);
              openSupport();
            },
          },
          {
            label: '兑换码',
            onClick: () => {
              setMenuOpen(false);
              setShowLicense(true);
              // 打开时重取：有期限的许可证可能在运行中到期
              invoke<LicenseSummary>('get_license_status')
                .then(setLicense)
                .catch(() => {});
            },
          },
          { label: '关于', onClick: handleShowAbout },
        ]}
      />

      <Overlay open={showSettings} onClose={() => setShowSettings(false)}>
        <SettingsDialog
          tab={settingsTab}
          onTabChange={setSettingsTab}
          appVersion={appVersion}
          inputMode={inputMode}
          layout={layout}
          windowScale={windowScale}
          onResetWindowScale={() => void resetScale()}
          switchingMode={switchingMode}
          globalEnabled={globalEnabled}
          togglingGlobal={togglingGlobal}
          elevated={elevated}
          closeBehavior={closeBehaviorPreference}
          interceptionInstalled={interceptionInstalled}
          interceptionIssue={interceptionIssue}
          ddHidInstalled={ddHidInstalled}
          hidStickPresent={hidStickPresent}
          autostartEnabled={sysInfo.autostart_enabled}
          autoEnableOnStart={autoEnableOnStart}
          runAsAdmin={sysInfo.run_as_admin}
          autoUpdate={autoUpdate}
          togglingAutostart={togglingAutostart}
          togglingRunAsAdmin={togglingRunAsAdmin}
          sound={sound}
          availableVoices={availableVoices}
          profiles={profileList}
          profileName={profileName}
          profileCount={profileList.length}
          isDefaultProfile={isDefaultProfile}
          signatureEnabled={grantsSignature(license)}
          signature={activeSignature ?? ''}
          onSignatureChange={(s) => void handleSignatureChange(s)}
          onClose={() => setShowSettings(false)}
          onSelectInputMode={(mode) => {
            setShowSettings(false);
            void selectInputMode(mode);
          }}
          hotkeys={hotkeys}
          hotkeyPolicy={keyPolicies.hotkey}
          hotkeyConflicts={{
            global_toggle: severityForKey(conflicts, hotkeys.global_toggle),
            global_stop: severityForKey(conflicts, hotkeys.global_stop),
            panel_toggle: severityForKey(conflicts, hotkeys.panel_toggle),
          }}
          onHotkeyChange={changeHotkeys}
          onToggleGlobal={() => void toggleGlobal()}
          onSetCloseBehavior={persistCloseBehavior}
          onToggleAutostart={() => void handleToggleAutostart()}
          onToggleAutoEnableOnStart={(next) => void handleToggleAutoEnableOnStart(next)}
          onToggleRunAsAdmin={() => void handleToggleRunAsAdmin()}
          onToggleAutoUpdate={handleToggleAutoUpdate}
          onSoundChange={persistSound}
          onPreviewSound={previewSound}
          onPickSoundAudio={pickSoundAudio}
          theme={theme}
          onThemeChange={persistTheme}
          onCreateProfile={() => {
            setShowSettings(false);
            void handleCreateProfile();
          }}
          onImportProfile={() => {
            setShowSettings(false);
            setShowImport(true);
          }}
          onSwitchProfile={(path) => void switchToProfile(path)}
          onRenameProfile={(name) => void handleRenameProfileByName(name)}
          onDeleteProfile={(name) => void handleDeleteProfileByName(name)}
          onExportProfile={(name) => void handleExportProfileByName(name)}
        />
      </Overlay>

      <Overlay open={showAgreement} onClose={() => setShowAgreement(false)} closeOnBackdrop={false}>
        <AgreementDialog onAgreed={handleAgreed} />
      </Overlay>

      <Overlay open={updateNotice !== null} onClose={() => setUpdateNotice(null)}>
        {updateNotice && (
          <UpdateNoticeDialog
            info={updateNotice.info}
            mode={updateNotice.mode}
            applying={applyingUpdate}
            onRestart={() => void handleApplyUpdate()}
            onClose={() => setUpdateNotice(null)}
          />
        )}
      </Overlay>

      <Overlay open={showJx3} onClose={() => setShowJx3(false)}>
        <Jx3ColumnDialog themeColor={theme.color} onClose={() => setShowJx3(false)} />
      </Overlay>

      <Overlay open={showTourCatalog} onClose={() => setShowTourCatalog(false)}>
        <TourCatalogDialog
          tours={TOURS}
          completed={tourProgress.completed}
          snapshot={tourSnapshot}
          onStart={(id) => {
            const tour = findTour(id);
            if (tour) startTour(tour);
          }}
          onClose={() => setShowTourCatalog(false)}
        />
      </Overlay>

      {activeTour && (
        <TourRunner
          tour={activeTour}
          host={tourHost}
          paused={
            showAgreement ||
            updateNotice !== null ||
            confirmOpen ||
            showImport ||
            importWelcome.pending
          }
          onExit={handleTourExit}
        />
      )}

      <Overlay open={showLicense} onClose={() => setShowLicense(false)}>
        <LicenseDialog
          summary={license}
          agreedVersion={agreedVersion}
          onActivated={(summary, version) => {
            setLicense(summary);
            if (version) {
              setAgreedVersion(version);
              settingsStore
                .set(MEMBERSHIP_AGREED_KEY, version)
                .then(() => settingsStore.save())
                .catch(() => {});
            }
          }}
          onOpenTarget={(target) => {
            setShowLicense(false);
            if (target === 'settings-profiles') handleShowSettings('profiles');
          }}
          onClose={() => setShowLicense(false)}
        />
      </Overlay>

      <Overlay open={showAbout} onClose={() => setShowAbout(false)}>
        <AboutDialog
          appearance={{ layout, scale: windowScale, maximized }}
          info={
            {
              appVersion,
              platform: sysInfo.platform,
              os_family: sysInfo.os_family,
              os_version: sysInfo.os_version,
              webview_version: sysInfo.webview_version,
              arch: sysInfo.arch,
              locale: sysInfo.locale,
              install_path: sysInfo.install_path,
              log_dir: sysInfo.log_dir,
              app_data_dir: sysInfo.app_data_dir,
              resources_ok: sysInfo.resources_ok,
              missing_resources: sysInfo.missing_resources,
              scheduler_hp_degraded: sysInfo.scheduler_hp_degraded,
            } satisfies AboutDialogInfo
          }
          onClose={() => setShowAbout(false)}
          onWriteMail={() => void writeMail()}
          onOpenDir={(kind) => {
            invoke('open_app_dir', { kind }).catch((err) => {
              toast.warning(`打开目录失败：${err}`);
            });
          }}
          onCopied={() => toast.success('已复制环境信息')}
          onGroupCopied={() => toast.success('已复制群号，在 QQ 中搜索加群')}
          onCopyFailed={(e) => toast.error(`复制失败：${e}`)}
        />
      </Overlay>

      <Overlay open={showRepair} onClose={() => setShowRepair(false)}>
        <RepairDialog
          elevated={elevated}
          autostartEnabled={sysInfo.autostart_enabled}
          inputMode={inputMode}
          interceptionInstalled={interceptionInstalled}
          ddHidInstalled={ddHidInstalled}
          onClose={() => setShowRepair(false)}
          onToast={(kind, msg) => {
            if (kind === 'success') toast.success(msg);
            else if (kind === 'warn') toast.warning(msg);
            else toast.error(msg);
          }}
          onInstallDriver={() => void handleInstallDriver()}
          onUninstallDriver={() => void handleUninstallDriver()}
          onUninstallDdHid={() => void handleUninstallDdHid()}
        />
      </Overlay>

      <Overlay open={showImport} onClose={() => setShowImport(false)}>
        <ImportDialog
          onClose={() => setShowImport(false)}
          onImport={async (request) => {
            await changeProfile(async () => {
              const profile = await invoke<Profile>('import_external_config', request);
              applyLoadedProfile(profile);
              await refreshProfileList();
              setShowImport(false);
              toast.success(`已导入配置「${profile.meta.name}」`);
            });
          }}
        />
      </Overlay>
    </div>
  );
}
