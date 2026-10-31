import type { LazyStore } from '@tauri-apps/plugin-store';
import { useCallback, useEffect, useState } from 'react';
import { completeImportWelcome, prepareImportWelcome } from './importWelcomeState';

/** 在协议同意前读取首装状态，避免协议升级或教程更新让老用户再次收到邀请。 */
export function useImportWelcome(store: LazyStore) {
  const [loaded, setLoaded] = useState(false);
  const [pending, setPending] = useState(false);

  useEffect(() => {
    let cancelled = false;
    prepareImportWelcome(store)
      .then((pending) => {
        if (!cancelled) setPending(pending);
      })
      .catch(() => {})
      .finally(() => {
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [store]);

  const complete = useCallback(async () => {
    try {
      await completeImportWelcome(store);
    } finally {
      setPending(false);
    }
  }, [store]);

  return { loaded, pending, complete };
}
