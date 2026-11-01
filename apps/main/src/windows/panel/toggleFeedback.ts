import type { KeyId } from './components/KeyCapture';

export interface ToggleFeedback {
  key: KeyId;
  started: boolean;
}

export function createToggleFeedbackHandler(options: {
  isReady: () => boolean;
  isEnabled: () => boolean;
  play: (key: KeyId, started: boolean) => void;
}) {
  return (feedback: ToggleFeedback) => {
    // 在事件抵达时读最新开关，停止之后到达的旧启停通知不能盖掉停止语音。
    if (options.isReady() && options.isEnabled()) options.play(feedback.key, feedback.started);
  };
}
