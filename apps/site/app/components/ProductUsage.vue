<script setup lang="ts">
interface Stats {
  product: 'flair-bloom';
  totals: { launches: number; deviceDays: number; devices: number; durationMs: number };
  stale: boolean;
}
const stats = ref<Stats>();
const failed = ref(false);
const config = useRuntimeConfig();
onMounted(async () => {
  try {
    const source = new URL(config.public.clientBootstrapUrl);
    const bootstrap = await $fetch<{ services?: { schemaVersion: number; publicStats: string } }>(
      source.href,
      { timeout: 12000, credentials: 'omit' },
    );
    if (
      bootstrap.services?.schemaVersion !== 1 ||
      bootstrap.services.publicStats !== '/api/v1/product-stats/flair-bloom'
    )
      throw new Error('无效配置');
    const value = await $fetch<Stats>(new URL(bootstrap.services.publicStats, source).href, {
      timeout: 5000,
      credentials: 'omit',
    });
    if (
      value.product !== 'flair-bloom' ||
      typeof value.stale !== 'boolean' ||
      !['launches', 'deviceDays', 'devices', 'durationMs'].every(
        (key) =>
          Number.isSafeInteger(value.totals?.[key as keyof Stats['totals']]) &&
          value.totals[key as keyof Stats['totals']] >= 0,
      )
    )
      throw new Error('无效统计');
    stats.value = value;
  } catch {
    failed.value = true;
  }
});
const values = computed(() => [
  { label: '累计启动', value: stats.value?.totals.launches },
  { label: '累计使用设备日', value: stats.value?.totals.deviceDays },
  { label: '用户数（按设备）', value: stats.value?.totals.devices },
  {
    label: '累计运行小时',
    value: stats.value ? Math.floor(stats.value.totals.durationMs / 3600000) : undefined,
  },
]);
</script>
<template>
  <div class="mt-6 text-(--ui-fg-muted)">
    <dl class="flex flex-wrap gap-x-6 gap-y-3 text-sm">
      <div v-for="item in values" :key="item.label">
        <dt>{{ item.label }}</dt>
        <dd class="m-0 mt-1 font-mono text-lg text-(--ui-fg)">
          {{ item.value === undefined ? '—' : item.value.toLocaleString('zh-CN') }}
        </dd>
      </div>
    </dl>
    <p class="mt-2 text-xs">
      按持久设备环境去重；每设备每日运行计一个设备日，不等于自然人数。{{
        failed ? '统计暂不可用。' : stats?.stale ? '当前显示缓存值。' : ''
      }}
      <NuxtLink to="/privacy" class="underline">协议与数据政策</NuxtLink>
    </p>
  </div>
</template>
