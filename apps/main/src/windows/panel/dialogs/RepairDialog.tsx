import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useRef, useState } from 'react';
import { WINDOWS_RESTART_HINT } from '../../../constants';
import Button from '../components/Button';
import { useConfirm } from '../components/ConfirmDialog';
import DialogShell from './DialogShell';
import './RepairDialog.css';

type Severity = 'info' | 'warn' | 'error';
type ItemStatus = 'ok' | 'orphan' | 'missing' | 'corrupted' | 'unknown';
type StepStatus = 'ok' | 'skipped' | 'failed' | 'pending_reboot';
type RepairCommand =
  | 'repair_dd_hid_residue'
  | 'repair_interception_residue'
  | 'repair_corrupted_profiles'
  | 'repair_clean_logs';

interface DiagnosticItem {
  id: string;
  category: string;
  label: string;
  severity: Severity;
  status: ItemStatus;
  detail: string;
  recommended_action: RepairCommand | null;
}

interface RepairReport {
  timestamp: string;
  items: DiagnosticItem[];
}

interface RepairStep {
  name: string;
  status: StepStatus;
  detail: string;
}

interface RepairOutcome {
  success: boolean;
  pending_reboot: boolean;
  summary: string;
  steps: RepairStep[];
  backup_dir: string | null;
}

interface DisplayIssue {
  id: string;
  title: string;
  detail: string;
  severity: Severity;
  action: RepairCommand | null;
}

type DriverStatus = 'installed' | 'pending_reboot' | 'not_installed';

interface Props {
  elevated: boolean;
  autostartEnabled: boolean;
  inputMode: string;
  interceptionInstalled: DriverStatus;
  ddHidInstalled: DriverStatus;
  onClose: () => void;
  onToast: (kind: 'success' | 'warn' | 'error', message: string) => void;
  onInstallDriver: () => void;
  onUninstallDriver: () => void;
  onUninstallDdHid: () => void;
}

const STEP_LABEL: Record<StepStatus, string> = {
  ok: '完成',
  skipped: '跳过',
  failed: '失败',
  pending_reboot: '待重启',
};

const ACTION_LABEL: Record<RepairCommand, string> = {
  repair_dd_hid_residue: '清理驱动残留',
  repair_interception_residue: '清理旧驱动残留',
  repair_corrupted_profiles: '修复配置',
  repair_clean_logs: '清理日志',
};

const ACTION_CONFIRM: Record<RepairCommand, string> = {
  repair_dd_hid_residue: '会清理 DD-HID 残留并自动备份，完成后可能需要重启电脑。',
  repair_interception_residue: '会清理旧输入驱动残留并自动备份，完成后需要重启电脑。',
  repair_corrupted_profiles: '会把损坏配置移到隔离目录，不会删除原文件。',
  repair_clean_logs: '会清理 7 天前的旧日志，崩溃日志会保留。',
};

const INPUT_MODE_LABEL: Record<string, string> = {
  sendinput: '通用模式',
  interception: '游戏模式',
  ddsimple: 'DD驱动',
  dd_hid: 'DDHID',
  hidstick: '外设模式',
};

function driverStatusLabel(status: DriverStatus): string {
  if (status === 'installed') return '已安装';
  if (status === 'pending_reboot') return '待重启';
  return '未安装';
}

function DriverFlag({ status }: { status: DriverStatus }) {
  const cls =
    status === 'installed'
      ? 'repair-flag--ok'
      : status === 'pending_reboot'
        ? 'repair-flag--warn'
        : 'repair-flag--off';
  return <span className={`repair-status-badge ${cls}`}>{driverStatusLabel(status)}</span>;
}

export default function RepairDialog({
  elevated,
  autostartEnabled,
  inputMode,
  interceptionInstalled,
  ddHidInstalled,
  onClose,
  onToast,
  onInstallDriver,
  onUninstallDriver,
  onUninstallDdHid,
}: Props) {
  const confirm = useConfirm();
  const [report, setReport] = useState<RepairReport | null>(null);
  const [scanning, setScanning] = useState(false);
  const [running, setRunning] = useState<RepairCommand | null>(null);
  const [exportingReport, setExportingReport] = useState(false);
  const [relaunching, setRelaunching] = useState(false);
  const [outcomes, setOutcomes] = useState<Record<RepairCommand, RepairOutcome | null>>({
    repair_dd_hid_residue: null,
    repair_interception_residue: null,
    repair_corrupted_profiles: null,
    repair_clean_logs: null,
  });

  // onToast 通常是父组件内联函数，每次重渲染都会换引用。把它锁进 ref，
  // useEffect / useCallback 依赖列表里只放稳定 ref，避免父组件的 toast 状态
  // 变化（包括自动关闭）触发 diagnose_environment 重复执行。
  const onToastRef = useRef(onToast);
  useEffect(() => {
    onToastRef.current = onToast;
  }, [onToast]);

  const scan = useCallback(async () => {
    setScanning(true);
    try {
      const r = await invoke<RepairReport>('diagnose_environment');
      setReport(r);
    } catch (e) {
      onToastRef.current('error', `诊断失败：${e}`);
    } finally {
      setScanning(false);
    }
  }, []);

  useEffect(() => {
    void scan();
  }, [scan]);

  const runRepair = useCallback(
    async (cmd: RepairCommand) => {
      if (running) return;
      const proceed = await confirm({
        title: ACTION_LABEL[cmd],
        description: ACTION_CONFIRM[cmd],
        confirmText: '继续修复',
        cancelText: '取消',
        tone:
          cmd === 'repair_clean_logs' || cmd === 'repair_corrupted_profiles' ? 'default' : 'danger',
      });
      if (!proceed) return;
      setRunning(cmd);
      try {
        const result = await invoke<RepairOutcome>(cmd);
        setOutcomes((prev) => ({ ...prev, [cmd]: result }));
        if (result.success) {
          if (result.pending_reboot) {
            onToastRef.current('warn', `${result.summary}\n${WINDOWS_RESTART_HINT}`);
          } else {
            onToastRef.current('success', result.summary);
          }
        } else {
          onToastRef.current('error', result.summary);
        }
        await scan();
      } catch (e) {
        onToastRef.current('error', `修复失败：${e}`);
      } finally {
        setRunning(null);
      }
    },
    [confirm, running, scan],
  );

  const exportDiagnosticReport = useCallback(async () => {
    if (exportingReport) return;
    setExportingReport(true);
    try {
      const path = await invoke<string>('export_dd_hid_diagnostic_report');
      onToastRef.current('success', `诊断报告已导出：${path}`);
    } catch (e) {
      onToastRef.current('error', `导出诊断报告失败：${e}`);
    } finally {
      setExportingReport(false);
    }
  }, [exportingReport]);

  // 手动以管理员身份重启：保留当前输入模式，新提权实例会接管。失败时才复位 loading，
  // 成功路径下本进程很快退出，无需复位。
  const relaunchAsAdmin = useCallback(async () => {
    if (relaunching) return;
    const ok = await confirm({
      title: '以管理员身份重启',
      description: '应用将以管理员权限重启，期间窗口会短暂关闭。授权 UAC 后会自动回到当前界面。',
      confirmText: '以管理员重启',
      cancelText: '取消',
    });
    if (!ok) return;
    setRelaunching(true);
    try {
      await invoke('relaunch_as_admin', { mode: inputMode });
    } catch (e) {
      onToastRef.current('error', `提权重启失败：${e}`);
      setRelaunching(false);
    }
  }, [confirm, relaunching, inputMode]);

  const displayIssues = buildDisplayIssues(report?.items ?? []);
  // 串口扫描是信息项，不进问题列表；每个口一行，放在驱动状态里
  const stickScan = report?.items.find((i) => i.id === 'hidstick.ports');
  const hasIssues = (report?.items ?? []).some((i) => i.severity !== 'info');

  const footerNode = (
    <>
      <Button
        variant="outline"
        tone="neutral"
        onClick={() => void exportDiagnosticReport()}
        loading={exportingReport}
      >
        导出诊断报告
      </Button>
      <Button variant="outline" onClick={() => void scan()} loading={scanning}>
        重新诊断
      </Button>
      <Button onClick={onClose}>关闭</Button>
    </>
  );

  return (
    <DialogShell
      className="repair-card"
      title="诊断修复"
      subtitle="自动检查影响安装和运行的常见问题。修复前会先备份。"
      labelId="repair-title"
      footer={footerNode}
      footerAlign="spread"
    >
      <div className="repair-body">
        <section className="repair-section repair-section--status">
          <h3 className="repair-section-title">运行状态</h3>
          <ul className="repair-status-list">
            <li>
              <span className="repair-status-key">管理员权限</span>
              <div className="repair-status-actions">
                <span
                  className={`repair-status-badge ${elevated ? 'repair-flag--ok' : 'repair-flag--off'}`}
                >
                  {elevated ? '已提权' : '普通用户'}
                </span>
                {!elevated && (
                  <Button
                    size="sm"
                    variant="outline"
                    tone="primary"
                    loading={relaunching}
                    onClick={() => void relaunchAsAdmin()}
                  >
                    以管理员重启
                  </Button>
                )}
              </div>
            </li>
            <li>
              <span className="repair-status-key">开机自启</span>
              <span
                className={`repair-status-badge ${autostartEnabled ? 'repair-flag--ok' : 'repair-flag--off'}`}
              >
                {autostartEnabled ? '已启用' : '未启用'}
              </span>
            </li>
            <li>
              <span className="repair-status-key">当前输入模式</span>
              <span className="repair-status-badge repair-flag--neutral">
                {INPUT_MODE_LABEL[inputMode] ?? inputMode}
              </span>
            </li>
          </ul>
        </section>

        <section className="repair-section repair-section--drivers">
          <h3 className="repair-section-title">驱动状态</h3>
          <p className="repair-status-hint">{WINDOWS_RESTART_HINT}</p>
          <ul className="repair-status-list">
            <li>
              <span className="repair-status-key">游戏模式驱动</span>
              <div className="repair-status-actions">
                <DriverFlag status={interceptionInstalled} />
                {interceptionInstalled === 'not_installed' && (
                  <Button size="sm" variant="outline" tone="primary" onClick={onInstallDriver}>
                    安装
                  </Button>
                )}
                {interceptionInstalled === 'installed' && (
                  <Button size="sm" variant="outline" tone="danger" onClick={onUninstallDriver}>
                    卸载
                  </Button>
                )}
              </div>
            </li>
            <li>
              <span className="repair-status-key">DDHID 驱动</span>
              <div className="repair-status-actions">
                <DriverFlag status={ddHidInstalled} />
                {ddHidInstalled === 'not_installed' && (
                  <span className="repair-status-hint">已屏蔽</span>
                )}
                {ddHidInstalled === 'installed' && (
                  <Button size="sm" variant="outline" tone="danger" onClick={onUninstallDdHid}>
                    卸载
                  </Button>
                )}
              </div>
            </li>
            <li className="repair-status-item--stack">
              <div className="repair-status-row">
                <span className="repair-status-key">外设驱动</span>
                <span
                  className={`repair-status-badge ${
                    !stickScan
                      ? 'repair-flag--neutral'
                      : stickScan.status === 'ok'
                        ? 'repair-flag--ok'
                        : 'repair-flag--off'
                  }`}
                >
                  {!stickScan
                    ? scanning
                      ? '扫描中'
                      : '未扫描'
                    : stickScan.status === 'ok'
                      ? '已识别'
                      : '未找到'}
                </span>
              </div>
              {stickScan && (
                <ul className="repair-port-list">
                  {stickScan.detail.split('；').map((line) => (
                    <li key={line}>{line}</li>
                  ))}
                </ul>
              )}
            </li>
          </ul>
        </section>

        {scanning && !report && <p className="repair-loading">正在诊断…</p>}

        {report && (
          <>
            <p className="repair-summary">
              {hasIssues ? (
                <span className="repair-summary--warn">检测到需要处理的项目</span>
              ) : (
                <span className="repair-summary--ok">未发现异常</span>
              )}
              <span className="repair-meta">扫描时间 {report.timestamp}</span>
            </p>
            {displayIssues.length === 0 ? (
              <p className="repair-empty">当前没有需要处理的问题。</p>
            ) : (
              <section className="repair-section">
                <ul className="repair-list">
                  {displayIssues.map((it) => (
                    <li key={it.id} className={`repair-item repair-item--${it.severity}`}>
                      <div className="repair-item-head">
                        <span className="repair-item-label">{it.title}</span>
                        <span className={`repair-flag repair-flag--${it.severity}`}>
                          {it.severity === 'error' ? '需要处理' : '建议处理'}
                        </span>
                      </div>
                      <p className="repair-item-detail">{it.detail}</p>
                      {it.action && (
                        <div className="repair-item-action">
                          <Button
                            size="sm"
                            variant="outline"
                            tone={it.severity === 'error' ? 'danger' : 'primary'}
                            loading={running === it.action}
                            disabled={running !== null && running !== it.action}
                            onClick={() => runRepair(it.action!)}
                          >
                            {ACTION_LABEL[it.action]}
                          </Button>
                        </div>
                      )}
                      {it.action && outcomes[it.action] && (
                        <RepairLog outcome={outcomes[it.action]!} />
                      )}
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </>
        )}
      </div>
    </DialogShell>
  );
}

function RepairLog({ outcome }: { outcome: RepairOutcome }) {
  return (
    <div className="repair-log">
      <p className={`repair-log-summary repair-log-summary--${outcome.success ? 'ok' : 'fail'}`}>
        {outcome.summary}
      </p>
      {outcome.pending_reboot && <p className="repair-status-hint">{WINDOWS_RESTART_HINT}</p>}
      <ul className="repair-log-steps">
        {outcome.steps.map((s, i) => (
          <li key={i} className={`repair-log-step repair-log-step--${s.status}`}>
            <span className="repair-log-step-name">{s.name}</span>
            <span className={`repair-flag repair-flag--step-${s.status}`}>
              {STEP_LABEL[s.status]}
            </span>
            <span className="repair-log-step-detail">{s.detail}</span>
          </li>
        ))}
      </ul>
      {outcome.backup_dir && <p className="repair-log-backup">备份目录：{outcome.backup_dir}</p>}
    </div>
  );
}

function buildDisplayIssues(items: DiagnosticItem[]): DisplayIssue[] {
  const byKey = new Map<string, DisplayIssue>();
  for (const item of items) {
    if (item.severity === 'info') continue;
    const issue = toDisplayIssue(item);
    const existing = byKey.get(issue.id);
    if (!existing) {
      byKey.set(issue.id, issue);
      continue;
    }
    if (severityRank(issue.severity) > severityRank(existing.severity)) {
      existing.severity = issue.severity;
    }
    if (!existing.action) {
      existing.action = issue.action;
    }
  }
  return Array.from(byKey.values()).sort(
    (a, b) => severityRank(b.severity) - severityRank(a.severity),
  );
}

function toDisplayIssue(item: DiagnosticItem): DisplayIssue {
  if (item.recommended_action === 'repair_dd_hid_residue' || item.id.startsWith('dd_hid.')) {
    return {
      id: 'dd_hid_residue',
      title: 'DD-HID 驱动残留',
      detail: '检测到驱动残留，可能导致重新安装失败。先清理残留，再重启电脑后重试。',
      severity: item.severity,
      action: item.recommended_action,
    };
  }
  if (
    item.recommended_action === 'repair_interception_residue' ||
    item.id.startsWith('interception.')
  ) {
    return {
      id: 'interception_residue',
      title: '旧输入驱动残留',
      detail: '检测到旧输入驱动残留，可能影响输入模式切换。清理后需要重启电脑。',
      severity: item.severity,
      action: item.recommended_action,
    };
  }
  // 槽位说明里有槽号与原因，后端那段比固定文案准，直接用
  if (item.id === 'hidstick.slots') {
    return {
      id: item.id,
      title: '外设驱动的输入被吞',
      detail: item.detail,
      severity: item.severity,
      action: null,
    };
  }
  if (item.id === 'prereq.resources') {
    return {
      id: item.id,
      title: '驱动文件异常',
      detail: '安装包里的驱动文件不完整或被修改。请重新安装最新版后再安装驱动。',
      severity: item.severity,
      action: null,
    };
  }
  if (item.id === 'prereq.hvci') {
    return {
      id: item.id,
      title: '需要关闭内存完整性',
      detail: 'Windows 的内存完整性会阻止这个驱动加载。关闭后重启电脑，再安装驱动。',
      severity: item.severity,
      action: null,
    };
  }
  if (item.id === 'prereq.sac') {
    return {
      id: item.id,
      title: 'Windows 安全策略可能拦截',
      detail: '当前安全策略可能拦截驱动安装程序。请先调整 Windows 安全设置后重试。',
      severity: item.severity,
      action: null,
    };
  }
  if (item.id === 'prereq.pending_reboot') {
    return {
      id: item.id,
      title: '需要重启电脑',
      detail: '系统有未完成的驱动或更新操作。请重启电脑后再安装驱动。',
      severity: item.severity,
      action: null,
    };
  }
  if (item.id === 'prereq.arch') {
    return {
      id: item.id,
      title: '当前系统不支持该驱动',
      detail: 'DD-HID 驱动只支持 64 位 Windows。',
      severity: item.severity,
      action: null,
    };
  }
  if (item.id === 'prereq.defender_exclusion') {
    return {
      id: item.id,
      title: '建议添加安全软件白名单',
      detail: '安全软件可能会拦截驱动安装文件。把安装目录加入白名单后再重试。',
      severity: item.severity,
      action: null,
    };
  }
  if (item.recommended_action === 'repair_corrupted_profiles') {
    return {
      id: 'corrupted_profiles',
      title: '配置文件损坏',
      detail: '检测到无法读取的配置文件。可以先修复配置，再重新打开应用。',
      severity: item.severity,
      action: item.recommended_action,
    };
  }
  if (item.recommended_action === 'repair_clean_logs') {
    return {
      id: 'old_logs',
      title: '日志占用较大',
      detail: '本地日志较多，可以清理旧日志释放空间。',
      severity: item.severity,
      action: item.recommended_action,
    };
  }
  return {
    id: item.id,
    title: item.label,
    detail: '检测到一个需要处理的问题。详细信息已写入诊断报告。',
    severity: item.severity,
    action: item.recommended_action,
  };
}

function severityRank(severity: Severity): number {
  if (severity === 'error') return 2;
  if (severity === 'warn') return 1;
  return 0;
}
