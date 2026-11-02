import type { KeyId } from './components/KeyCapture';
import { keyToken } from './keyToken.ts';

export interface OutputStep {
  key: KeyId;
  interval_ms: number;
}

export interface SequenceRule {
  trigger_key: KeyId;
  steps: OutputStep[];
  mode?: 'hold' | 'toggle';
  stop_key?: KeyId | null;
}

export const MAX_OUTPUT_STEPS = 256;

export function isSimpleRule(rule: SequenceRule): boolean {
  return (
    rule.steps.length === 1 &&
    keyToken(rule.trigger_key) === keyToken(rule.steps[0].key) &&
    (rule.mode !== 'toggle' ||
      !rule.stop_key ||
      keyToken(rule.stop_key) === keyToken(rule.trigger_key))
  );
}

export function outputKeys(rule: Pick<SequenceRule, 'steps'>): KeyId[] {
  return [...new Map(rule.steps.map((step) => [keyToken(step.key), step.key])).values()];
}

export function validateSteps(steps: OutputStep[], min: number, max: number): string | undefined {
  if (steps.length === 0 || steps.length > MAX_OUTPUT_STEPS)
    return `队列需要 1～${MAX_OUTPUT_STEPS} 个步骤`;
  const invalid = steps.findIndex(
    (step) =>
      !Number.isInteger(step.interval_ms) || step.interval_ms < min || step.interval_ms > max,
  );
  if (invalid >= 0) return `第 ${invalid + 1} 步间隔需要在 ${min}～${max}ms 之间`;
  return undefined;
}
