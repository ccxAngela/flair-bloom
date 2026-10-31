import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useRef, useState } from 'react';
import Button from '../components/Button';
import type { KeyId } from '../components/KeyCapture';
import './dialog-base.css';
import './ImportDialog.css';

interface FoundConfig {
  path: string;
  source_app: string;
  display_name: string;
  source_profile_index: number | null;
}

interface ImportPreview {
  source_app: string;
  suggested_name: string;
  rule_count: number;
  skipped_count: number;
  interval_ms: number;
  interval_max_ms: number;
  warnings: string[];
  global_toggle: KeyId | null;
  global_stop: KeyId | null;
  panel_toggle: KeyId | null;
}

interface Props {
  onClose: () => void;
  onImported: (profileName: string) => void;
}

function keyLabel(key: KeyId | null): string {
  if (!key) return '';
  if (key.kind === 'keyboard') {
    const vk = key.code as number;
    if (vk >= 0x70 && vk <= 0x7b) return `F${vk - 0x6f}`;
    if (vk >= 0x30 && vk <= 0x39) return String.fromCharCode(vk);
    if (vk >= 0x41 && vk <= 0x5a) return String.fromCharCode(vk);
    return `VK_0x${vk.toString(16).toUpperCase()}`;
  }
  const labels: Record<string, string> = {
    left: '鼠标左键',
    right: '鼠标右键',
    middle: '鼠标中键',
    x1: '鼠标侧键1',
    x2: '鼠标侧键2',
  };
  return labels[key.code as string] ?? String(key.code);
}

export default function ImportDialog({ onClose, onImported }: Props) {
  const [scanning, setScanning] = useState(false);
  const [found, setFound] = useState<FoundConfig[]>([]);
  const [scanDone, setScanDone] = useState(false);
  const [customDir, setCustomDir] = useState('');
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [selectedSourceProfile, setSelectedSourceProfile] = useState<number | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState('');
  const [profileName, setProfileName] = useState('');
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState('');
  const customInputRef = useRef<HTMLInputElement>(null);
  const previewRequest = useRef(0);

  const runScan = useCallback(async (dirs: string[]) => {
    setScanning(true);
    try {
      const result = await invoke<FoundConfig[]>('scan_import_configs', { dirs });
      setFound((prev) => {
        const existing = new Set(prev.map(configIdentity));
        return [...prev, ...result.filter((f) => !existing.has(configIdentity(f)))];
      });
    } catch {
      // 静默失败，展示空列表即可
    } finally {
      setScanning(false);
      setScanDone(true);
    }
  }, []);

  // 初始扫描
  useEffect(() => {
    void runScan([]);
  }, [runScan]);

  async function loadPreview(
    path: string,
    sourceProfileIndex: number | null = null,
    suggestedName?: string,
  ) {
    const request = ++previewRequest.current;
    setSelectedPath(path);
    setSelectedSourceProfile(sourceProfileIndex);
    setPreview(null);
    setPreviewError('');
    setProfileName('');
    setPreviewLoading(true);
    try {
      const p = await invoke<ImportPreview>('preview_import', { path, sourceProfileIndex });
      if (request !== previewRequest.current) return;
      setPreview(p);
      setProfileName(suggestedName ?? p.suggested_name);
    } catch (e) {
      if (request !== previewRequest.current) return;
      setPreviewError(String(e));
    } finally {
      if (request === previewRequest.current) setPreviewLoading(false);
    }
  }

  async function handleCustomScan() {
    const dir = customDir.trim();
    if (!dir) return;
    // 先尝试作为文件直接预览
    if (/\.(json|ini)$/i.test(dir)) {
      await loadPreview(dir);
      return;
    }
    setScanning(true);
    try {
      const result = await invoke<FoundConfig[]>('scan_import_configs', { dirs: [dir] });
      if (result.length === 0) {
        setPreviewError('该目录下未找到已知格式的配置文件');
      } else {
        setFound((prev) => {
          const existing = new Set(prev.map(configIdentity));
          return [...prev, ...result.filter((f) => !existing.has(configIdentity(f)))];
        });
        setPreviewError('');
      }
    } catch (e) {
      setPreviewError(String(e));
    } finally {
      setScanning(false);
    }
  }

  async function handleImport() {
    if (!selectedPath || !preview || !profileName.trim()) return;
    setImporting(true);
    setImportError('');
    try {
      const profile = await invoke<{ meta: { name: string } }>('import_external_config', {
        path: selectedPath,
        profileName: profileName.trim(),
        sourceProfileIndex: selectedSourceProfile,
      });
      onImported(profile.meta.name);
    } catch (e) {
      setImportError(String(e));
    } finally {
      setImporting(false);
    }
  }

  const canImport =
    !!selectedPath &&
    !!preview &&
    preview.rule_count > 0 &&
    profileName.trim().length > 0 &&
    !importing &&
    !previewLoading;

  return (
    <div className="modal-mask" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal import-modal">
        <h2>导入外部配置</h2>
        <p className="modal-desc">
          支持丐帮高手、衍天高手、极简按键和小叽按键。多个方案可分别选择导入，转换差异会在预览中说明。
        </p>

        {/* 扫描结果列表 */}
        <div className="import-section">
          <div className="import-section-title">
            <span>自动扫描结果</span>
            <button
              className="rescan-btn"
              onClick={() => {
                setFound([]);
                setScanDone(false);
                void runScan([]);
              }}
              disabled={scanning}
            >
              {scanning ? '扫描中…' : '重新扫描'}
            </button>
          </div>
          <div className="found-list">
            {found.length === 0 && scanDone && !scanning && (
              <p className="found-empty">
                未在桌面、下载及衍天高手默认配置目录找到已知格式的配置文件
              </p>
            )}
            {found.length === 0 && scanning && <p className="found-empty scanning">正在扫描…</p>}
            {found.map((f) => (
              <button
                key={configIdentity(f)}
                className={`found-item${selectedPath === f.path && selectedSourceProfile === f.source_profile_index ? ' selected' : ''}`}
                onClick={() => loadPreview(f.path, f.source_profile_index, f.display_name)}
              >
                <span className="found-app">{f.source_app}</span>
                <span className="found-name">{f.display_name}</span>
                <span className="found-path" title={f.path}>
                  {f.path}
                </span>
              </button>
            ))}
          </div>
        </div>

        {/* 手动输入目录/文件路径 */}
        <div className="import-section">
          <div className="import-section-title">手动指定</div>
          <div className="custom-dir-row">
            <input
              ref={customInputRef}
              className="custom-dir-input"
              type="text"
              placeholder="粘贴配置文件路径或所在目录路径…"
              value={customDir}
              onChange={(e) => setCustomDir(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void handleCustomScan();
              }}
            />
            <Button
              size="sm"
              variant="outline"
              tone="neutral"
              disabled={!customDir.trim() || scanning}
              onClick={handleCustomScan}
            >
              扫描
            </Button>
          </div>
        </div>

        {/* 预览区 */}
        {(previewLoading || previewError || preview) && (
          <div className="import-section import-preview-section">
            <div className="import-section-title">配置预览</div>
            {previewLoading && <p className="found-empty scanning">解析中…</p>}
            {previewError && <p className="import-preview-error">{previewError}</p>}
            {preview && !previewLoading && (
              <div className="import-preview">
                <div className="preview-grid">
                  <span className="preview-label">来源软件</span>
                  <span className="preview-value">{preview.source_app}</span>
                  <span className="preview-label">规则数量</span>
                  <span className="preview-value">
                    {preview.rule_count} 条规则
                    {preview.skipped_count > 0 && (
                      <span className="preview-warn">
                        （跳过 {preview.skipped_count} 条不支持或超出上限的规则）
                      </span>
                    )}
                  </span>
                  <span className="preview-label">连发间隔</span>
                  <span className="preview-value">
                    {preview.interval_ms === preview.interval_max_ms
                      ? preview.interval_ms
                      : `${preview.interval_ms}–${preview.interval_max_ms}`}{' '}
                    ms
                  </span>
                  {preview.global_toggle && (
                    <>
                      <span className="preview-label">全局开启键</span>
                      <span className="preview-value">{keyLabel(preview.global_toggle)}</span>
                    </>
                  )}
                  {preview.global_stop && (
                    <>
                      <span className="preview-label">全局停止键</span>
                      <span className="preview-value">{keyLabel(preview.global_stop)}</span>
                    </>
                  )}
                  {preview.panel_toggle && (
                    <>
                      <span className="preview-label">面板显隐键</span>
                      <span className="preview-value">{keyLabel(preview.panel_toggle)}</span>
                    </>
                  )}
                </div>
                {preview.warnings.map((warning) => (
                  <p key={warning} className="preview-warn">
                    {warning}
                  </p>
                ))}
                <div className="profile-name-row">
                  <label className="profile-name-label">配置名称</label>
                  <input
                    className="profile-name-input"
                    type="text"
                    value={profileName}
                    onChange={(e) => setProfileName(e.target.value)}
                    placeholder="请输入配置名称"
                    maxLength={40}
                  />
                </div>
              </div>
            )}
          </div>
        )}

        {importError && <p className="import-error">{importError}</p>}

        <div className="modal-actions">
          <Button variant="ghost" tone="neutral" onClick={onClose}>
            取消
          </Button>
          <Button
            variant="solid"
            tone="primary"
            disabled={!canImport}
            loading={importing}
            onClick={handleImport}
          >
            导入
          </Button>
        </div>
      </div>
    </div>
  );
}

function configIdentity(config: FoundConfig): string {
  return `${config.path}#${config.source_profile_index ?? ''}`;
}
