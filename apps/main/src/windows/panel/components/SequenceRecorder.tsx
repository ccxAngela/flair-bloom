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
  const [remainingSeconds, setRemainingSeconds] = useState<number>();
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
    const deadline = performance.now() + 3000;
    setPending(true);
    setSnapshot(undefined);
    setMessage(undefined);
    const tick = () => {
      if (!mounted.current || attempt.current !== ticket) return;
      const remainingMs = deadline - performance.now();
      const seconds = Math.max(0, Math.ceil(remainingMs / 1000));
      setRemainingSeconds(seconds || undefined);
      if (seconds > 0) {
        countdown.current = setTimeout(tick, Math.max(1, remainingMs - (seconds - 1) * 1000));
        return;
      }
      countdown.current = undefined;
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
    };
    tick();
  };

  const stop = async () => {
    if (pending) {
      attempt.current += 1;
      clearTimeout(countdown.current);
      setRemainingSeconds(undefined);
      setPending(false);
      setMessage('已取消录制');
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

  const status = pending
    ? remainingSeconds !== undefined
      ? `${remainingSeconds} 秒后开始录制，请切换窗口`
      : '正在开始录制…'
    : message || snapshot?.notice;

  return (
    <section className="sequence-recording" aria-label="录制连发序列">
      <div className="sequence-actions">
        <span>结束键</span>
        <KeyCapture
          policy={{ ...policy, mouse: [] }}
          value={stopKey}
          onReject={onReject}
          onChange={(key) => key && !snapshot?.recording && !pending && setStopKey(key)}
        />
        <button type="button" disabled={pending || snapshot?.recording} onClick={start}>
          录制序列
        </button>
        {(pending || snapshot?.recording) && (
          <button type="button" onClick={() => void stop()}>
            {pending ? '取消录制' : '结束录制'}
          </button>
        )}
      </div>
      {snapshot?.recording && (
        <p role="status">
          正在录制 {snapshot.steps.length} 步，按 {keyLabel(stopKey)} 结束。
        </p>
      )}
      {status && <p role="status">{status}</p>}
      {snapshot && !snapshot.recording && snapshot.steps.length > 0 && (
        <>
          <p>{snapshot.steps.map((step) => keyLabel(step.key)).join(' → ')}</p>
          <div className="sequence-actions">
            <span>末步到首步间隔</span>
            <IntervalInput value={tail} min={min} max={max} onChange={setTail} />
            <button
              type="button"
              onClick={() => {
                setSnapshot(undefined);
                setMessage(undefined);
              }}
            >
              取消
            </button>
            <button
              type="button"
              onClick={() => {
                onRecorded(
                  snapshot.steps.map((step, index) =>
                    index === snapshot.steps.length - 1
                      ? { ...step, interval_ms: tail }
                      : { ...step },
                  ),
                );
                setSnapshot(undefined);
              }}
            >
              采用
            </button>
          </div>
        </>
      )}
    </section>
  );
}
