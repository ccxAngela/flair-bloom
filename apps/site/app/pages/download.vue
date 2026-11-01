<script setup lang="ts">
import type { InstallerKind, ReleaseAsset } from '~/composables/useReleases';

useHead({ title: '下载气质花按键助手 FlairBloom' });

const route = useRoute();
const { latest, status } = useReleases();

// 默认发 exe；分享 ?type=msi 给需要走组策略或静默安装的人
const kind = computed<InstallerKind>(() => (route.query.type === 'msi' ? 'msi' : 'nsis'));
const asset = computed(() => latest.value?.assets.find((item) => item.kind === kind.value));
const other = computed(() => latest.value?.assets.find((item) => item.kind !== kind.value));
const unavailable = computed(
  () => status.value === 'error' || (status.value === 'success' && !asset.value),
);

const started = ref(false);

// 安装包按 application/octet-stream 下发，跳过去浏览器只弹下载、不离开本页。
// 只触发一次：清单或查询参数后续变化不应再弹第二个下载
watch(
  asset,
  (next) => {
    if (!next || started.value) return;
    started.value = true;
    window.location.assign(next.url);
  },
  { immediate: true },
);

function label(item: ReleaseAsset): string {
  return item.kind === 'msi' ? 'MSI 安装包' : 'EXE 安装包';
}
</script>

<template>
  <main class="flex min-h-dvh items-center justify-center px-4 py-12">
    <div class="ui-glow-card w-full max-w-md p-8 text-center">
      <img src="/icon.png" alt="" width="96" height="96" class="mx-auto size-24 drop-shadow-lg" />

      <template v-if="asset">
        <h1 class="mt-6 text-xl font-semibold text-(--ui-fg-strong)">
          正在下载气质花 {{ latest?.tag }}
        </h1>
        <p class="mt-2 text-sm text-(--ui-fg-muted)">
          {{ label(asset) }} · {{ formatSize(asset.size) }} · Windows 10 / 11（64 位）
        </p>
        <a
          :href="asset.url"
          class="mt-6 inline-flex items-center gap-2 rounded-(--ui-radius) bg-(--ui-primary) px-5 py-3 font-semibold text-(--ui-primary-fg) shadow-(--ui-shadow-primary) hover:opacity-90"
        >
          <XIcon name="ph:download-simple" class="size-5" />
          没有开始？点这里下载
        </a>
        <p v-if="other" class="mt-4 text-sm">
          <a
            :href="other.url"
            class="text-(--ui-fg-muted) underline-offset-2 hover:text-(--ui-fg) hover:underline"
            >改下 {{ label(other) }}</a
          >
        </p>
      </template>

      <template v-else-if="unavailable">
        <h1 class="mt-6 text-xl font-semibold text-(--ui-fg-strong)">暂时读不到最新版本</h1>
        <p class="mt-2 text-sm text-(--ui-fg-muted)">可以先到 GitHub 发布页下载。</p>
        <a
          :href="`${GITHUB_RELEASES_URL}/latest`"
          target="_blank"
          rel="noopener"
          class="mt-6 inline-flex items-center gap-2 rounded-(--ui-radius) bg-(--ui-primary) px-5 py-3 font-semibold text-(--ui-primary-fg) hover:opacity-90"
        >
          前往 GitHub 下载
          <XIcon name="ph:arrow-up-right" class="size-5" />
        </a>
      </template>

      <template v-else>
        <h1 class="mt-6 text-xl font-semibold text-(--ui-fg-strong)">正在读取最新版本</h1>
        <p class="mt-2 inline-flex items-center gap-2 text-sm text-(--ui-fg-muted)">
          <XIcon name="ph:spinner" class="size-4 animate-spin" />
          马上开始下载
        </p>
      </template>

      <p class="mt-8 border-t border-(--ui-primary)/20 pt-4 text-sm">
        <NuxtLink
          to="/"
          class="text-(--ui-primary) underline-offset-2 hover:underline"
          >功能介绍、上手步骤与更新公告</NuxtLink
        >
      </p>
      <p class="mt-3 text-sm text-(--ui-fg-muted)">
        使用问题或建议？
        <NuxtLink
          :to="{ path: '/', hash: '#support' }"
          class="text-(--ui-primary) underline-offset-2 hover:underline"
        >加入产品QQ群</NuxtLink>
      </p>
    </div>
  </main>
</template>
