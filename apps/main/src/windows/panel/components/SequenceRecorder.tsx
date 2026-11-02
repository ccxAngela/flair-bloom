import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import KeyCapture, {
  keyLabel,
  keyboardKey,
  type KeyId,
  type SlotPolicy,
  type CaptureReject,
} from './KeyCapture';
import IntervalInput from './IntervalInput';
import type { OutputStep } from '../outputSequence';

interface Snapshot {
  session_id: number;
  recording: boolean;
  steps: OutputStep[];
  notice?: string;
}
interface Props {
  policy: SlotPolicy;
  min: number;
  max: number;
  onReject: (info: CaptureReject) => void;
  onRecorded: (steps: OutputStep[]) => void;
}

export default function SequenceRecorder({ policy, min, max, onReject, onRecorded }: Props) {
  const [stopKey, setStopKey] = useState<KeyId>(keyboardKey(123));
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [message, setMessage] = useState<string>();
  const [pending, setPending] = useState(false);
  const [tail, setTail] = useState(1000);
  const owned = useRef<number | undefined>(undefined);
  const countdown = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const mounted = useRef(true);
  const attempt = useRef(0);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      attempt.current += 1;
      clearTimeout(countdown.current);
      if (owned.current !== undefined)
        void invoke('stop_recording', { sessionId: owned.current }).catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (!snapshot?.recording) return;
    let cancelled = false;
    const poll = () =>
      invoke<Snapshot>('get_recording', { sessionId: snapshot.session_id })
        .then((next) => {
          if (!cancelled) setSnapshot(next);
        })
        .catch((error) => {
          if (!cancelled) {
            setMessage(String(error));
            setSnapshot(undefined);
          }
        });
    const timer = setInterval(() => void poll(), 150);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [snapshot?.recording, snapshot?.session_id]);

  const start = () => {
    const ticket = ++attempt.current;
    setPending(true);
    setSnapshot(undefined);
    setMessage('3 秒后开始录制，请切到需要录制的窗口。原有连发将在录制开始时停止。');
    countdown.current = setTimeout(() => {
      void invoke<Snapshot>('start_recording', { stopKey })
        .then((next) => {
          if (!mounted.current || attempt.current !== ticket) {
            void invoke('stop_recording', { sessionId: next.session_id }).catch(() => {});
            return;
          }
          owned.current = next.session_id;
          setSnapshot(next);
          setMessage(undefined);
        })
        .catch((error) => {
          if (mounted.current && attempt.current === ticket) setMessage(String(error));
        })
        .finally(() => {
          if (mounted.current && attempt.current === ticket) setPending(false);
        });
    }, 3000);
  };

  const stop = async () => {
    if (pending) {
      attempt.current += 1;
      clearTimeout(countdown.current);
      setPending(false);
      setMessage('已取消录制倒计时');
      return;
    }
    if (owned.current === undefined) return;
    try {
      const next = await invoke<Snapshot>('stop_recording', { sessionId: owned.current });
      if (mounted.current) setSnapshot(next);
    } catch (error) {
      if (mounted.current) setMessage(String(error));
    }
  };

  return (
    <section className="sequence-recording" aria-label="录制输出队列">
      <p>
        录制普通键盘点按，最多 5 分钟／256
        步。按压时长统一按点按播放；鼠标按键可手动添加，本期不支持组合键。
      </p>
      <div className="sequence-actions">
        <span>结束录制键</span>
        <KeyCapture
          policy={{ ...policy, mouse: [] }}
          value={stopKey}
          onReject={onReject}
          onChange={(key) => key && !snapshot?.recording && !pending && setStopKey(key)}
        />
        <button type="button" disabled={pending || snapshot?.recording} onClick={start}>
          录制队列
        </button>
        {(pending || snapshot?.recording) && (
          <button type="button" onClick={() => void stop()}>
            {pending ? '取消倒计时' : '结束录制'}
          </button>
        )}
      </div>
      {snapshot?.recording && (
        <p role="status">
          正在录制 {snapshot.steps.length} 步，按 {keyLabel(stopKey)} 结束。
        </p>
      )}
      {(message || snapshot?.notice) && <p role="status">{message || snapshot?.notice}</p>}
      {snapshot && !snapshot.recording && snapshot.steps.length > 0 && (
        <>
          <p>
            录制预览：{snapshot.steps.map((step) => keyLabel(step.key)).join(' → ')}
            。请检查顺序和间隔后采用。
          </p>
          <div className="sequence-actions">
            <span>末步到首步间隔</span>
            <IntervalInput value={tail} min={min} max={max} onChange={setTail} />
            <button
              type="button"
              onClick={() =>
                onRecorded(
                  snapshot.steps.map((step, index) =>
                    index === snapshot.steps.length - 1
                      ? { ...step, interval_ms: tail }
                      : { ...step },
                  ),
                )
              }
            >
              采用已录步骤
            </button>
          </div>
        </>
      )}
    </section>
  );
}
