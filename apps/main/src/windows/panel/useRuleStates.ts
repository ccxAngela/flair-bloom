import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

/** 引擎 `get_rule_states`：paused 是被同组按住的长按插队而暂时让位的规则，仍算开启。 */
interface RuleStates {
  running: string[];
  paused: string[];
}

export interface RuleStateSets {
  /** running ∪ paused：暂停 / 恢复不改变集合，界面仍显示为已开启。 */
  active: Set<string>;
  paused: Set<string>;
}

const EMPTY = new Set<string>();

function sameMembers(prev: Set<string>, ids: string[]): boolean {
  return prev.size === ids.length && ids.every((id) => prev.has(id));
}

/**
 * 面板与浮窗共用的规则状态轮询。每拍只调一次 `get_rule_states`：运行与暂停取自引擎同一把锁下
 * 的快照，分两次调用拼会错拍。内容不变时返回原集合，避免每拍重渲染。停用时清空，不留残影。
 */
export function useRuleStates(enabled: boolean, intervalMs: number): RuleStateSets {
  const [active, setActive] = useState<Set<string>>(EMPTY);
  const [paused, setPaused] = useState<Set<string>>(EMPTY);

  useEffect(() => {
    if (!enabled) {
      setActive((prev) => (prev.size === 0 ? prev : EMPTY));
      setPaused((prev) => (prev.size === 0 ? prev : EMPTY));
      return;
    }
    let cancelled = false;
    const poll = () => {
      invoke<RuleStates>('get_rule_states')
        .then(({ running, paused: pausedIds }) => {
          if (cancelled) return;
          const ids = [...running, ...pausedIds];
          setActive((prev) => (sameMembers(prev, ids) ? prev : new Set(ids)));
          setPaused((prev) => (sameMembers(prev, pausedIds) ? prev : new Set(pausedIds)));
        })
        .catch(() => {});
    };
    poll();
    const timer = setInterval(poll, intervalMs);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [enabled, intervalMs]);

  return { active, paused };
}
