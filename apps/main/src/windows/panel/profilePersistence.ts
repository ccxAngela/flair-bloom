export interface ProfileWriteTicket {
  generation: number;
  name: Promise<string>;
}

export function createProfilePersistence<T>(
  write: (name: string, value: T) => Promise<void>,
  onError: (error: unknown) => void,
  delayMs = 500,
) {
  let generation = 0;
  let changing = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pending: { ticket: ProfileWriteTicket; value: T } | undefined;
  let writing: Promise<void> = Promise.resolve();
  const updates = new Set<Promise<unknown>>();

  function isCurrent(ticket: ProfileWriteTicket) {
    return ticket.generation === generation;
  }

  function capture(name: string | Promise<string>): ProfileWriteTicket {
    const target = Promise.resolve(name);
    void target.catch(() => {});
    return { generation, name: target };
  }

  function track<R>(operation: Promise<R>): Promise<R> {
    updates.add(operation);
    void operation.then(
      () => updates.delete(operation),
      () => updates.delete(operation),
    );
    return operation;
  }

  async function flush() {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    // 热键／规则校验回调可能继续提交快照，先等它们完成，再取最后一份。
    while (updates.size > 0) await Promise.all(updates);
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    const job = pending;
    pending = undefined;
    if (job) {
      const task = writing
        .catch(() => {})
        .then(async () => {
          const name = await job.ticket.name;
          await write(name, job.value);
        });
      writing = task;
      try {
        await task;
      } catch (error) {
        // 失败时保留可重试快照；后续新编辑优先，不恢复过期内容。
        if (!pending && isCurrent(job.ticket)) pending = job;
        throw error;
      }
    } else {
      await writing;
    }
  }

  function schedule(ticket: ProfileWriteTicket, value: T) {
    if (!isCurrent(ticket)) return;
    pending = { ticket, value };
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(() => void flush().catch(onError), delayMs);
  }

  async function change<R>(action: () => Promise<R>): Promise<R> {
    if (changing) throw new Error('配置正在切换，请稍后再试');
    changing = true;
    try {
      await flush();
      generation += 1;
      return await action();
    } finally {
      changing = false;
    }
  }

  function dispose() {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    pending = undefined;
    generation += 1;
  }

  return {
    capture,
    isCurrent,
    track,
    schedule,
    flush,
    change,
    dispose,
    isChanging: () => changing,
  };
}
