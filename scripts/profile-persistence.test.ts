import assert from 'node:assert/strict';
import test from 'node:test';
import { createProfilePersistence } from '../apps/main/src/windows/panel/profilePersistence.ts';

function deferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

test('切换前保存防抖窗口内的最后编辑，旧回调不能写进新配置', async () => {
  const writes: Array<[string, string]> = [];
  const persistence = createProfilePersistence<string>(async (name, value) => {
    writes.push([name, value]);
  }, assert.fail);
  const old = persistence.capture('旧配置');
  persistence.schedule(old, '第一次编辑');
  persistence.schedule(old, '最后编辑');
  await persistence.change(async () => {
    assert.deepEqual(writes, [['旧配置', '最后编辑']]);
  });
  persistence.schedule(old, '迟到的旧快照');
  persistence.schedule(persistence.capture('导入的新配置'), '新编辑');
  await persistence.flush();
  assert.deepEqual(writes, [
    ['旧配置', '最后编辑'],
    ['导入的新配置', '新编辑'],
  ]);
  persistence.dispose();
});

test('热键校验尚未返回时导入，等回调提交保存后再激活新配置', async () => {
  const validated = deferred();
  const events: string[] = [];
  const persistence = createProfilePersistence<string>(async (name, value) => {
    events.push(`保存:${name}:${value}`);
  }, assert.fail);
  const ticket = persistence.capture('旧配置');
  persistence.track(validated.promise.then(() => persistence.schedule(ticket, '新热键')));
  const changing = persistence.change(async () => {
    events.push('激活导入配置');
  });
  assert.equal(persistence.isChanging(), true);
  assert.deepEqual(events, []);
  validated.resolve();
  await changing;
  assert.deepEqual(events, ['保存:旧配置:新热键', '激活导入配置']);
  persistence.dispose();
});

test('默认配置另存未完成时切换，等待实际副本名称并保存到副本', async () => {
  const forked = deferred<string>();
  const events: string[] = [];
  const persistence = createProfilePersistence<string>(async (name, value) => {
    events.push(`${name}:${value}`);
  }, assert.fail);
  persistence.schedule(persistence.capture(forked.promise), '编辑后的规则');
  const changing = persistence.change(async () => {
    events.push('切换');
  });
  assert.deepEqual(events, []);
  forked.resolve('我的配置2');
  await changing;
  assert.deepEqual(events, ['我的配置2:编辑后的规则', '切换']);
  persistence.dispose();
});

test('磁盘写入未完成时不得激活新配置', async () => {
  const writing = deferred();
  const started = deferred();
  let activated = false;
  const persistence = createProfilePersistence<string>(async () => {
    started.resolve();
    await writing.promise;
  }, assert.fail);
  persistence.schedule(persistence.capture('旧配置'), '规则');
  const flush = persistence.flush();
  await started.promise;
  const change = persistence.change(async () => {
    activated = true;
  });
  assert.equal(activated, false);
  writing.resolve();
  await Promise.all([flush, change]);
  assert.equal(activated, true);
  persistence.dispose();
});

test('默认配置另存失败不会留下不可重试的保存票据', async () => {
  const forked = deferred<string>();
  let nativeUpdates = 0;
  let failures = 0;
  let writes = 0;
  const persistence = createProfilePersistence<string>(async () => {
    writes++;
  }, assert.fail);
  const ticket = persistence.capture(forked.promise);
  persistence.track(
    ticket.name
      .then(() => {
        nativeUpdates++;
        persistence.schedule(ticket, '编辑');
      })
      .catch(() => {
        failures++;
      }),
  );
  const changing = persistence.change(async () => {});
  forked.reject(new Error('无法创建配置'));
  await changing;
  assert.equal(failures, 1);
  assert.equal(nativeUpdates, 0);
  assert.equal(writes, 0);
  persistence.schedule(persistence.capture('重试另存成功'), '新编辑');
  await persistence.flush();
  assert.equal(writes, 1);
  persistence.dispose();
});

test('保存失败阻止切换，保留快照供下次重试', async () => {
  let fail = true;
  let activated = false;
  const writes: string[] = [];
  const persistence = createProfilePersistence<string>(async (_, value) => {
    if (fail) throw new Error('磁盘不可写');
    writes.push(value);
  }, assert.fail);
  persistence.schedule(persistence.capture('旧配置'), '未丢失的编辑');
  await assert.rejects(
    persistence.change(async () => {
      activated = true;
    }),
    /磁盘不可写/,
  );
  assert.equal(activated, false);
  assert.equal(persistence.isChanging(), false);
  fail = false;
  await persistence.change(async () => {
    activated = true;
  });
  assert.equal(activated, true);
  assert.deepEqual(writes, ['未丢失的编辑']);
  persistence.dispose();
});

test('同一时间只允许一项配置变更，失败后可以再次切换', async () => {
  const activated = deferred();
  const started = deferred();
  const persistence = createProfilePersistence<string>(async () => {}, assert.fail);
  const first = persistence.change(async () => {
    started.resolve();
    await activated.promise;
    throw new Error('加载失败');
  });
  await started.promise;
  await assert.rejects(
    persistence.change(async () => {}),
    /正在切换/,
  );
  activated.resolve();
  await assert.rejects(first, /加载失败/);
  await persistence.change(async () => {});
  persistence.dispose();
});
