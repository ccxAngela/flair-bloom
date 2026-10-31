export interface ImportWelcomeStore {
  get<T>(key: string): Promise<T | undefined>;
  set(key: string, value: unknown): Promise<void>;
  save(): Promise<void>;
}

export async function prepareImportWelcome(store: ImportWelcomeStore): Promise<boolean> {
  const [agreed, seen, savedPending] = await Promise.all([
    store.get<boolean>('agreed'),
    store.get<boolean>('import_welcome_seen'),
    store.get<boolean>('import_welcome_pending'),
  ]);
  const pending = seen !== true && (agreed !== true || savedPending === true);
  if (pending && savedPending !== true) {
    await store.set('import_welcome_pending', true);
    await store.save();
  }
  return pending;
}

export async function completeImportWelcome(store: ImportWelcomeStore): Promise<void> {
  await store.set('import_welcome_seen', true);
  await store.set('import_welcome_pending', false);
  await store.save();
}
