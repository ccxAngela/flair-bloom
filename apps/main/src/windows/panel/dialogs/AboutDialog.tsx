import { useEffect, useState } from 'react';
import { APP_NAME, AUTHOR_EMAIL } from '../../../constants';
import Button from '../components/Button';
import { collectLiveEnvironment, type LiveEnvironment } from '../environment';
import DialogShell from './DialogShell';
import './AboutDialog.css';

const INPUT_MODE_LABELS: Record<string, string> = {
  sendinput: '通用模式',
  interception: '游戏模式',
  ddsimple: 'DD驱动',
  hidstick: '外设模式',
};

export interface AboutDialogInfo {
  appVersion: string;
  platform: string;
  os_family: string;
  os_version: string;
  webview_version: string;
  arch: string;
  locale: string;
  install_path: string;
  log_dir: string;
  app_data_dir: string;
  resources_ok: boolean;
  missing_resources: string[];
  scheduler_hp_degraded: boolean;
}

interface Props {
  info: AboutDialogInfo;
  appearance: { layout: string; scale: number; maximized: boolean };
  onClose: () => void;
  onWriteMail: () => void;
  onOpenDir: (kind: 'install' | 'data' | 'log' | 'drivers') => void;
  onCopied: () => void;
  onCopyFailed: (err: unknown) => void;
}

function InfoRow({ label, value }: { label: string; value: string | React.ReactNode }) {
  return (
    <li>
      <span className="about-key">{label}</span>
      <span className="about-value">{value}</span>
    </li>
  );
}

function DirRow({ label, onOpen }: { label: string; onOpen: () => void }) {
  return (
    <li>
      <span className="about-key">{label}</span>
      <span className="about-value about-value--with-action">
        <Button size="sm" variant="outline" onClick={onOpen}>
          打开
        </Button>
      </span>
    </li>
  );
}

export default function AboutDialog({
  info,
  appearance,
  onClose,
  onWriteMail,
  onOpenDir,
  onCopied,
  onCopyFailed,
}: Props) {
  const [copying, setCopying] = useState(false);
  const [refreshing, setRefreshing] = useState(true);
  const [environment, setEnvironment] = useState<LiveEnvironment | null>(null);

  useEffect(() => {
    let disposed = false;
    void collectLiveEnvironment().then((result) => {
      if (!disposed) {
        setEnvironment(result);
        setRefreshing(false);
      }
    });
    return () => {
      disposed = true;
    };
  }, []);

  async function handleRefresh() {
    setRefreshing(true);
    try {
      setEnvironment(await collectLiveEnvironment());
    } finally {
      setRefreshing(false);
    }
  }

  async function handleCopy() {
    if (copying) return;
    setCopying(true);
    try {
      const current = await collectLiveEnvironment();
      setEnvironment(current);
      const payload = {
        report_version: 2,
        app: { name: APP_NAME, version: info.appVersion },
        platform: info.platform,
        os_family: info.os_family,
        os_version: info.os_version,
        webview_version: info.webview_version,
        arch: info.arch,
        locale: info.locale,
        install_path: info.install_path,
        log_dir: info.log_dir,
        app_data_dir: info.app_data_dir,
        resources_ok: info.resources_ok,
        missing_resources: info.missing_resources,
        scheduler_hp_degraded: info.scheduler_hp_degraded,
        ...current.status,
        captured_at: current.captured_at,
        display: current.display,
        frontend: current.frontend,
        appearance,
        collection_errors: current.errors,
      };
      await navigator.clipboard.writeText(JSON.stringify(payload, null, 2));
      onCopied();
    } catch (e) {
      onCopyFailed(e);
    } finally {
      setCopying(false);
    }
  }

  const runtime = environment?.status;
  const display = environment?.display;
  const panel = display?.windows.find((w) => w.label === 'panel');
  const float = display?.windows.find((w) => w.label === 'float');
  const webviewVersion =
    display?.runtime_version || runtime?.webview_version || info.webview_version;
  const percent = (factor: number) => `${Math.round(factor * 1000) / 10}%`;
  const size = (value: { width: number; height: number } | null | undefined) =>
    value ? `${value.width} × ${value.height}` : '未取得';
  const resourceHealth = runtime ?? info;

  const headerNode = (
    <>
      <h2 className="about-title">
        {APP_NAME}
        <span className="about-ver">{info.appVersion ? `v${info.appVersion}` : '版本加载中…'}</span>
      </h2>
      <p className="about-tagline">加强花椒油！！！加强紫武区！！！</p>
      {!resourceHealth.resources_ok && (
        <p className="about-warn-banner">
          检测到缺失资源：{resourceHealth.missing_resources.join('、') || '未知'}。
          可能被杀毒软件误删，建议重新安装应用。
        </p>
      )}
    </>
  );

  const footerNode = (
    <>
      <Button variant="outline" onClick={handleRefresh} loading={refreshing} disabled={copying}>
        刷新
      </Button>
      <Button variant="outline" onClick={handleCopy} loading={copying} disabled={refreshing}>
        复制环境信息
      </Button>
      <Button onClick={onClose}>关闭</Button>
    </>
  );

  return (
    <DialogShell className="about-card" headerContent={headerNode} footer={footerNode}>
      <div className="about-body">
        <section className="about-section">
          <p className="about-section-label">联系作者</p>
          <ul className="about-list">
            <li>
              <span className="about-key">邮箱</span>
              <span className="about-value about-value--with-action">
                <span className="about-email">{AUTHOR_EMAIL}</span>
                <Button size="sm" variant="outline" tone="primary" onClick={onWriteMail}>
                  写邮件
                </Button>
              </span>
            </li>
          </ul>
        </section>

        <section className="about-section">
          <p className="about-section-label">运行环境</p>
          <ul className="about-list">
            <InfoRow
              label="操作系统"
              value={
                <>
                  {runtime?.os_version || info.os_version || info.platform || '—'}
                  {info.os_family && info.os_family !== info.platform ? ` (${info.os_family})` : ''}
                </>
              }
            />
            <InfoRow label="主机架构" value={info.arch || '—'} />
            <InfoRow label="语言区域" value={info.locale || '—'} />
            <InfoRow
              label="高精度计时"
              value={(runtime ?? info).scheduler_hp_degraded ? '已降级' : '可用'}
            />
            <InfoRow
              label="管理员权限"
              value={runtime ? (runtime.elevated ? '已获得' : '未获得') : '未取得'}
            />
            <InfoRow
              label="当前输入通道"
              value={
                runtime ? INPUT_MODE_LABELS[runtime.input_mode] || runtime.input_mode : '未取得'
              }
            />
            <InfoRow
              label="快速启动配置"
              value={
                display?.fast_startup_configured == null
                  ? '未取得'
                  : display.fast_startup_configured
                    ? '已开启'
                    : '已关闭'
              }
            />
            <InfoRow
              label="WebView2"
              value={
                !webviewVersion ? (
                  <span className="about-flag about-flag--warn">未检测到</span>
                ) : (
                  webviewVersion
                )
              }
            />
          </ul>
        </section>

        <section className="about-section">
          <p className="about-section-label">显示环境</p>
          <ul className="about-list">
            <InfoRow
              label="系统文字大小"
              value={
                display?.text_scale_percent != null
                  ? `${display.text_scale_percent}%（应用忽略）`
                  : '未取得'
              }
            />
            <InfoRow
              label="应用布局 / 缩放"
              value={`${appearance.layout === 'horizontal' ? '横版' : '竖版'} · ${percent(appearance.scale)}${appearance.maximized ? ' · 最大化' : ''}`}
            />
            <InfoRow
              label="当前屏幕缩放"
              value={panel?.scale_factor != null ? percent(panel.scale_factor) : '未取得'}
            />
            <InfoRow
              label="面板 / 浮窗尺寸"
              value={`${size(panel?.client_physical)} / ${size(float?.client_physical)} px`}
            />
            <InfoRow
              label="页面视口"
              value={`${size(environment?.frontend.viewport_css)} CSS px`}
            />
            <InfoRow
              label="页面像素比"
              value={environment ? String(environment.frontend.device_pixel_ratio) : '未取得'}
            />
            <InfoRow
              label="WebView 实际缩放"
              value={panel?.webview ? percent(panel.webview.zoom_factor) : '未取得'}
            />
            <InfoRow
              label="WebView 像素缩放"
              value={panel?.webview ? percent(panel.webview.rasterization_scale) : '未取得'}
            />
            {display?.monitors.map((monitor, index) => (
              <InfoRow
                key={`${monitor.name}-${index}`}
                label={`显示器 ${index + 1}${monitor.primary ? '（主）' : ''}`}
                value={`${size(monitor.size_physical)} · ${percent(monitor.scale_factor)}`}
              />
            ))}
          </ul>
          {!!(environment?.errors.length || display?.errors.length) && (
            <p className="about-warn-banner">部分环境信息未取得，错误详情会一并复制。</p>
          )}
        </section>

        <section className="about-section">
          <p className="about-section-label">目录</p>
          <ul className="about-list">
            <DirRow label="安装目录" onOpen={() => onOpenDir('install')} />
            <DirRow label="数据目录" onOpen={() => onOpenDir('data')} />
            <DirRow label="日志目录" onOpen={() => onOpenDir('log')} />
            {info.os_family === 'windows' && (
              <DirRow label="驱动目录" onOpen={() => onOpenDir('drivers')} />
            )}
          </ul>
        </section>
      </div>
    </DialogShell>
  );
}
