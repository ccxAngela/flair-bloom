import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import KeyCapture, { type CaptureReject, type SlotPolicy } from './KeyCapture';
import IntervalInput from './IntervalInput';
import { MAX_OUTPUT_STEPS, validateSteps, type OutputStep } from '../outputSequence';
import SequenceRecorder from './SequenceRecorder';

interface DraftStep extends OutputStep {
  editor_id: string;
}
const editableStep = (step: OutputStep): DraftStep => ({ ...step, editor_id: crypto.randomUUID() });
const outputSteps = (steps: DraftStep[]): OutputStep[] =>
  steps.map(({ key, interval_ms }) => ({ key, interval_ms }));

interface Props {
  steps: OutputStep[];
  policy: SlotPolicy;
  min: number;
  max: number;
  onReject: (info: CaptureReject) => void;
  onSave: (steps: OutputStep[]) => Promise<void>;
  onClose: () => void;
}

export default function SequenceEditor({
  steps,
  policy,
  min,
  max,
  onReject,
  onSave,
  onClose,
}: Props) {
  const [draft, setDraft] = useState(() => steps.map(editableStep));
  const error = validateSteps(draft, min, max);
  const [preview, setPreview] = useState<string>();
  const [previewPending, setPreviewPending] = useState(false);
  const [previewMessage, setPreviewMessage] = useState<string>();
  const [savePending, setSavePending] = useState(false);
  const [saveMessage, setSaveMessage] = useState<string>();
  const saving = useRef(false);
  const previewOwner = useRef<string | undefined>(undefined);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      if (previewOwner.current)
        void invoke('stop_preview', { ruleId: previewOwner.current }).catch(() => {});
    };
  }, []);
  useEffect(() => {
    if (!preview) return;
    let cancelled = false;
    const timer = setInterval(() => {
      void invoke<{ running: string[]; progress: { rule_id: string; error?: string }[] }>(
        'get_rule_states',
      )
        .then((state) => {
          if (cancelled) return;
          const failure = state.progress.find((p) => p.rule_id === preview)?.error;
          if (!state.running.includes(preview)) {
            setPreview(undefined);
            setPreviewMessage(failure || '单次试播已结束');
          }
        })
        .catch((error) => {
          if (!cancelled) setPreviewMessage(String(error));
        });
    }, 150);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [preview]);
  const playOnce = async () => {
    setPreviewPending(true);
    try {
      const id = await invoke<string>('preview_output', { steps: outputSteps(draft) });
      if (!alive.current) {
        void invoke('stop_preview', { ruleId: id }).catch(() => {});
        return;
      }
      previewOwner.current = id;
      setPreview(id);
      setPreviewMessage('正在向前台窗口试播一次，关闭编辑器或全局停止可中断');
    } catch (error) {
      if (alive.current) setPreviewMessage(String(error));
    } finally {
      if (alive.current) setPreviewPending(false);
    }
  };
  const move = (index: number, offset: number) =>
    setDraft((previous) => {
      const next = [...previous];
      [next[index], next[index + offset]] = [next[index + offset], next[index]];
      return next;
    });

  const save = async () => {
    if (saving.current) return;
    saving.current = true;
    setSavePending(true);
    setSaveMessage(undefined);
    try {
      await onSave(outputSteps(draft));
      if (alive.current) onClose();
    } catch (error) {
      if (alive.current) setSaveMessage(`保存失败，草稿已保留：${String(error)}`);
    } finally {
      saving.current = false;
      if (alive.current) setSavePending(false);
    }
  };

  return (
    <fieldset
      className="sequence-editor"
      disabled={savePending}
      role="dialog"
      aria-label="编辑输出队列"
      data-tour="sequence-editor"
      onDragStart={(event) => event.stopPropagation()}
    >
      <p>依次点按各步，最后一步结束后回到第一步。间隔包含本步按压时间。</p>
      <ol className="sequence-steps">
        {draft.map((step, index) => (
          <li key={step.editor_id} className="sequence-step">
            <span>{index + 1}</span>
            <KeyCapture
              policy={policy}
              value={step.key}
              onReject={onReject}
              onChange={(key) =>
                key &&
                setDraft((previous) =>
                  previous.map((item, i) => (i === index ? { ...item, key } : item)),
                )
              }
            />
            <IntervalInput
              value={step.interval_ms}
              min={min}
              max={max}
              onChange={(interval_ms) =>
                setDraft((previous) =>
                  previous.map((item, i) => (i === index ? { ...item, interval_ms } : item)),
                )
              }
            />
            <button
              type="button"
              disabled={index === 0}
              aria-label={`上移第 ${index + 1} 步`}
              onClick={() => move(index, -1)}
            >
              ↑
            </button>
            <button
              type="button"
              disabled={index === draft.length - 1}
              aria-label={`下移第 ${index + 1} 步`}
              onClick={() => move(index, 1)}
            >
              ↓
            </button>
            <button
              type="button"
              disabled={draft.length >= MAX_OUTPUT_STEPS}
              onClick={() =>
                setDraft((previous) => [
                  ...previous.slice(0, index + 1),
                  editableStep(step),
                  ...previous.slice(index + 1),
                ])
              }
            >
              复制
            </button>
            <button
              type="button"
              disabled={draft.length === 1}
              aria-label={`删除第 ${index + 1} 步`}
              onClick={() => setDraft((previous) => previous.filter((_, i) => i !== index))}
            >
              删除
            </button>
          </li>
        ))}
      </ol>
      <div className="sequence-actions">
        <button
          type="button"
          disabled={draft.length >= MAX_OUTPUT_STEPS}
          onClick={() =>
            setDraft((previous) => [...previous, editableStep(previous[previous.length - 1])])
          }
        >
          ＋ 添加步骤
        </button>
        <span>
          {draft.length} 步 · 一轮至少 {draft.reduce((sum, step) => sum + step.interval_ms, 0)}ms
        </span>
        <button type="button" onClick={onClose}>
          取消
        </button>
        <button
          type="button"
          disabled={!!error || !!preview || previewPending}
          onClick={() => void playOnce()}
        >
          试播一次
        </button>
        {preview && (
          <button
            type="button"
            onClick={() => {
              void invoke('stop_preview', { ruleId: preview }).then(() => {
                setPreview(undefined);
                setPreviewMessage('试播已停止');
              });
            }}
          >
            停止试播
          </button>
        )}
        <button type="button" disabled={!!error} onClick={() => void save()}>
          {savePending ? '保存中…' : '保存队列'}
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
      {saveMessage && <p role="alert">{saveMessage}</p>}
      <p>试播需先启用全局开关，并会停止当前连发。发送到当前前台窗口。</p>
      {previewMessage && <p role="status">{previewMessage}</p>}
      <SequenceRecorder
        policy={policy}
        min={min}
        max={max}
        onReject={onReject}
        onRecorded={(recorded) => setDraft(recorded.map(editableStep))}
      />
    </fieldset>
  );
}
