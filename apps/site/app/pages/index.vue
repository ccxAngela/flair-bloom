<script setup lang="ts">
import { useHashAnchor, useTheme } from '@xwink/ui';

const { releases, latest, status } = useReleases();
const { isDark, preference: themePreference, toggle: toggleTheme } = useTheme();
const groupCopyStatus = ref('');

async function copyGroupNumber() {
  try {
    await navigator.clipboard.writeText(PRODUCT_QQ_GROUP);
    groupCopyStatus.value = '已复制群号，在 QQ 中搜索加群';
  } catch {
    groupCopyStatus.value = '复制失败，请手动复制上方群号';
  }
}

const themeModes = [
  { value: 'light', label: '浅色', icon: 'ph:sun-dim' },
  { value: 'dark', label: '深色', icon: 'ph:moon' },
  { value: 'auto', label: '跟随系统', icon: 'ph:desktop' },
] as const;

// 预渲染时读不到本地存的明暗档，服务端只能按「跟随系统」出选中态；水合时 class 不一致 Vue 不纠正会残留。
// 选中高亮等挂载后再给，挂载前两端都是「无选中」，挂载后是正常的响应式更新
const mounted = ref(false);
onMounted(() => {
  mounted.value = true;
});

const nsis = computed(() => latest.value?.assets.find((asset) => asset.kind === 'nsis'));
const msi = computed(() => latest.value?.assets.find((asset) => asset.kind === 'msi'));
const unavailable = computed(
  () => status.value === 'error' || (status.value === 'success' && !nsis.value),
);

const { color: brandColor, select: selectBrand } = useBrandColor();

// 右轨刻度从上到下就是版面顺序；轨上只放得下几个字，label 要短
const railSections = [
  { id: 'download', label: '下载' },
  { id: 'features', label: '白皮书' },
  { id: 'rumors', label: '骚话' },
  { id: 'changelog', label: '公告' },
  { id: 'support', label: '支持' },
];

// 公告异步读完会把下方内容撑开：锚点跳完得纠偏，滚走后过期的 hash 要清掉。
// 归页面管而不交给 XScrollRail：轨还藏着，靠一个看不见的组件兜这件事读代码的人想不到
useHashAnchor();

// 同一列表里的卡片错峰进场；封顶是因为长列表后面的卡片等太久，滚到时反而像卡住
const stagger = (index: number) => 120 + Math.min(index, 4) * 90;
const currentBrand = computed(() => brandPresets.find((preset) => preset.color === brandColor.value));

// 每张卡占几格，宽窄交替排出错落；顺序与 highlights 一一对应，教学轴另外用
// col-start / row-span 钉在右侧那一列。三列下正好铺满四行，不留空格
const HIGHLIGHT_SPANS = [
  'sm:col-span-2', // 颜值能打：要够宽才能把 20 颗色球排成一行
  '', // 配置灵活
  '', // 多形态布局
  'sm:col-span-2', // 三模式覆盖
];

// 白皮书区的版面次序（行优先）：第一张亮点 → 教学轴 → 左列自上而下。
// 数组顺序是数据顺序、不是看到的顺序，教学轴又钉在右侧跨三行，
// 照数组下标错峰会看着乱跳，所以进场与跑马灯都按这套次序走，两个波才同向
const TOUR_ORDER = 1;
const HIGHLIGHT_ORDER = [0, 2, 3, 4];

// 跑马灯按 --ui-i 排队轮流亮。默认周期是按一轮 6 格调的，张数变了得跟着放长，
// 否则几张卡会同时亮
const GLOW_STAGGER = 2.4;
const glowCycle = `${((highlights.length + 1) * GLOW_STAGGER).toFixed(1)}s`;
</script>

<template>
  <div>
  <!-- 打印时整页换成 PrintKit 的海报加说明书排版，屏幕版不参与打印 -->
  <PrintKit />
  <div class="min-h-dvh print:hidden">
    <!-- 覆盖式顶栏：浮在首屏极光之上，随滚动渐进成毛玻璃；高度与首屏上边距、锚点偏移（scroll-mt-16）对应 -->
    <XGlassHeader>
      <div class="relative mx-auto flex h-16 max-w-5xl items-center gap-3 px-4 sm:px-6">
        <a href="#top" class="flex min-w-0 items-center gap-2 font-semibold text-(--ui-fg-strong)">
          <img src="/icon.png" alt="" class="size-7 rounded-md" />
          <span class="truncate text-(--ui-primary)">气质花 FlairBloom</span>
        </a>
        <nav class="ms-auto hidden items-center gap-5 text-sm text-(--ui-fg-muted) sm:flex">
          <a href="#features" class="hover:text-(--ui-fg)">功能介绍</a>
          <a href="#changelog" class="hover:text-(--ui-fg)">更新公告</a>
          <a href="#support" class="hover:text-(--ui-fg)">支持&帮助</a>
        </nav>
        <!-- 顶栏只留一键明暗翻转；三档选择与门派色块在功能区卡片里。图标取决于本地存的档位，同样只在客户端渲染 -->
        <ClientOnly>
          <button
            type="button"
            class="ms-auto flex size-9 cursor-pointer items-center justify-center rounded-(--ui-radius) text-(--ui-fg-muted) hover:bg-(--ui-primary)/10 hover:text-(--ui-primary) sm:ms-0"
            :title="isDark ? '切换到浅色' : '切换到深色'"
            :aria-label="isDark ? '切换到浅色' : '切换到深色'"
            @click="toggleTheme"
          >
            <XIcon :name="isDark ? 'ph:moon' : 'ph:sun-dim'" class="size-5" />
          </button>
          <template #fallback><div class="ms-auto size-9 sm:ms-0" /></template>
        </ClientOnly>
      </div>
    </XGlassHeader>

    <main id="top">
      <section id="download" class="scroll-mt-16 mx-auto max-w-5xl px-4 pt-30 pb-16 sm:px-6 sm:pt-36">
        <div class="flex flex-col items-start gap-10 md:flex-row md:items-center">
          <div class="flex-1">
            <XReveal>
              <p class="text-sm font-medium text-(--ui-primary)">
                PVE 打本按键小助手 · 极速连发 · 降低延迟 · 提高输出
              </p>
              <h1 class="mt-3 text-4xl font-bold tracking-tight text-(--ui-fg-strong) sm:text-5xl">
                气质花按键助手
              </h1>
            </XReveal>
            <XReveal :delay="120">
              <p class="mt-4 max-w-xl text-lg text-(--ui-fg-muted)">
                手搓技能卡 GCD、一键宏\武学助手 FFF 启动、多段宏丝滑切换……<br/>让手指<PinyinGag gag="(duì xiàng)" real="(shǒu zhǐ)" />歇一会儿，DPS也能更高点。
              </p>
            </XReveal>

            <XReveal :delay="240">
            <div class="mt-8 flex flex-wrap items-center gap-3">
              <a
                v-if="nsis"
                :href="nsis.url"
                class="inline-flex items-center gap-2 rounded-(--ui-radius) border border-transparent bg-(--ui-primary) px-5 py-3 font-semibold text-(--ui-primary-fg) shadow-(--ui-shadow-primary) hover:opacity-90"
              >
                <XIcon name="ph:download-simple" class="size-5" />
                下载 {{ latest?.tag }}
              </a>
              <span
                v-else-if="!unavailable"
                class="inline-flex items-center gap-2 rounded-(--ui-radius) border border-(--ui-border) px-5 py-3 text-(--ui-fg-muted)"
              >
                <XIcon name="ph:spinner" class="size-5 animate-spin" />
                正在读取最新版本
              </span>
              <a
                v-else
                :href="`${REPOSITORY}/releases/latest`"
                target="_blank"
                rel="noopener"
                class="inline-flex items-center gap-2 rounded-(--ui-radius) border border-transparent bg-(--ui-primary) px-5 py-3 font-semibold text-(--ui-primary-fg) hover:opacity-90"
              >
                前往 GitHub 下载
                <XIcon name="ph:arrow-up-right" class="size-5" />
              </a>
              <a
                v-if="msi"
                :href="msi.url"
                class="text-sm text-(--ui-fg-muted) underline-offset-2 hover:text-(--ui-fg) hover:underline"
              >
                MSI 安装包
              </a>
            </div>

            <!-- 清单在浏览器里异步读，读到之前用同样长度的不可见文本占住行高，免得读完把下面整页往下推 -->
            <p class="mt-3 text-xs text-(--ui-fg-muted)">
              Windows 10 / 11（64 位）<template v-if="nsis">
                · {{ formatSize(nsis.size) }} · 发布于
                {{ formatDate(latest?.publishedAt ?? '') }} ·
                <a
                  :href="nsis.githubUrl"
                  target="_blank"
                  rel="noopener"
                  class="underline-offset-2 hover:underline"
                  >GitHub 下载</a
                ></template
              ><span v-else-if="!unavailable" class="invisible" aria-hidden="true">
                · 0.0 MB · 发布于 0000-00-00 · GitHub 下载</span
              >
            </p>
            <p class="mt-3 text-sm text-(--ui-fg-muted)">
              使用问题或建议？
              <a href="#support" class="text-(--ui-primary) underline-offset-2 hover:underline">
                加入产品QQ群
              </a>
            </p>
            </XReveal>
          </div>

          <XReveal :delay="360" class="mx-auto">
            <img
              src="/icon.png"
              alt="气质花图标"
              width="220"
              height="220"
              class="size-40 drop-shadow-xl sm:size-56"
            />
          </XReveal>
        </div>
      </section>

      <section id="features" class="scroll-mt-16">
        <div class="mx-auto max-w-5xl px-4 py-16 sm:px-6">
          <XReveal>
            <h2 class="text-2xl font-semibold text-(--ui-fg-strong)">APP白皮书</h2>
          </XReveal>
          <p class="mt-2 text-sm text-(--ui-fg-muted)">王婆卖瓜，广告位招租</p>
          <!-- 错落 bento：三列网格里宽窄交替（2+1、1+1、2+1），教学轴竖起来占右侧一列三行。
               色板卡横躺两列，20 颗色球一行排完，不必再被拉成一根空柱子 -->
          <ul
            class="ui-glow-marquee mt-8 grid gap-4 sm:grid-cols-2 md:grid-cols-3"
            :style="{ '--ui-glow-cycle': glowCycle }"
          >
            <!-- 教学轴有两种形态：中间档（sm~md）横着通栏排三步，md 起立在右侧窄列自上而下 -->
            <!-- row-start 不能省：只给 col-start 的话它是「列定行不定」的项，放置游标推进到第三列
                 就不回退，后面每张卡都会被挤到第二行起排，左上角空出一格 -->
            <li class="sm:col-span-2 md:col-span-1 md:col-start-3 md:row-span-3 md:row-start-1">
              <XReveal :delay="stagger(TOUR_ORDER)" class="h-full">
                <div class="ui-glow-card flex h-full flex-col p-6" :style="{ '--ui-i': TOUR_ORDER }">
                  <div
                    class="flex flex-col gap-1 sm:flex-row sm:items-baseline sm:gap-3 md:flex-col md:gap-1"
                  >
                    <h3 class="shrink-0 text-lg font-semibold text-(--ui-fg-strong)">
                      {{ tourGuide.title }}
                    </h3>
                    <p class="text-sm text-(--ui-fg-muted)">{{ tourGuide.text }}</p>
                  </div>
                  <div class="relative mt-6 flex-1">
                    <span class="tour-rail-bubble" aria-hidden="true">
                      <XIcon name="ph:chat-circle-dots" class="size-6" />
                    </span>
                    <!-- md 起三行等高，气泡才好按 1/6、3/6、5/6 在节点之间跳 -->
                    <ol
                      class="grid h-full gap-5 sm:grid-cols-3 sm:gap-4 md:grid-cols-1 md:grid-rows-3 md:gap-5"
                    >
                      <li
                        v-for="(step, index) in tourSteps"
                        :key="step.title"
                        class="relative ps-14 sm:ps-0 sm:text-center md:ps-14 md:text-start"
                      >
                        <!-- 连线分两种：竖排自上而下接下一个节点，横排自左而右接上一个节点 -->
                        <span
                          v-if="index < tourSteps.length - 1"
                          class="tour-rail-stem"
                          aria-hidden="true"
                        />
                        <span v-if="index > 0" class="tour-rail-seg" aria-hidden="true" />
                        <span
                          class="absolute start-0 top-0 flex size-10 items-center justify-center rounded-full border-2 border-(--ui-primary) bg-(--ui-bg) font-semibold text-(--ui-primary) sm:static sm:mx-auto md:absolute md:mx-0"
                          >{{ index + 1 }}</span
                        >
                        <h4
                          class="pt-2 font-semibold text-(--ui-fg-strong) sm:mt-3 sm:pt-0 md:mt-0 md:pt-2"
                        >
                          {{ step.title }}
                        </h4>
                        <p class="mt-1 text-sm text-(--ui-fg-muted)">{{ step.text }}</p>
                      </li>
                    </ol>
                  </div>
                </div>
              </XReveal>
            </li>
            <!-- 一张循环铺完六张亮点卡：宽窄按 HIGHLIGHT_SPANS 交替，门派色那张就地多挂一组控件 -->
            <li
              v-for="(item, index) in highlights"
              :key="item.title"
              :class="HIGHLIGHT_SPANS[index]"
            >
              <XReveal :delay="stagger(HIGHLIGHT_ORDER[index])" class="h-full">
                <div
                  class="ui-glow-card flex h-full flex-col gap-4 p-6 sm:flex-row"
                  :style="{ '--ui-i': HIGHLIGHT_ORDER[index] }"
                >
                  <HighlightIcon :name="item.icon" />
                  <div>
                    <h3 class="text-lg font-semibold text-(--ui-fg-strong)">{{ item.title }}</h3>
                    <ul class="mt-2 space-y-1 text-sm text-(--ui-fg-muted)">
                      <li v-for="point in item.points" :key="point" class="relative ps-4 before:absolute before:start-0 before:top-2 before:size-1.5 before:rounded-full before:bg-(--ui-primary)/60">
                        {{ point }}
                      </li>
                      <li v-if="item.icon === 'palette'" class="relative ps-4 before:absolute before:start-0 before:top-2 before:size-1.5 before:rounded-full before:bg-(--ui-primary)/60">
                        当前外观是<span class="text-(--ui-fg)"
                          >{{ currentBrand?.sect }} · {{ currentBrand?.name
                          }}{{ mounted && isDark ? '（黑化版）' : '' }}</span
                        >。
                      </li>
                    </ul>
                    <!-- 横躺两格，20 颗色球一行排得开，卡片不用往高里长 -->
                    <div
                      v-if="item.icon === 'palette'"
                      class="mt-4 flex flex-wrap items-center gap-x-4 gap-y-3"
                    >
                      <div class="flex items-center gap-2">
                        <button
                          v-for="mode in themeModes"
                          :key="mode.value"
                          type="button"
                          class="flex size-8 cursor-pointer items-center justify-center rounded-full border-2 transition-all hover:scale-110 hover:text-(--ui-primary)"
                          :class="
                            mounted && themePreference === mode.value
                              ? 'border-(--ui-primary) bg-(--ui-primary)/15 text-(--ui-primary)'
                              : 'border-transparent bg-(--ui-primary)/5 text-(--ui-fg-muted)'
                          "
                          :title="mode.label"
                          :aria-label="mode.label"
                          :aria-pressed="mounted && themePreference === mode.value"
                          @click="themePreference = mode.value"
                        >
                          <XIcon :name="mode.icon" class="size-4" />
                        </button>
                      </div>
                      <div class="flex flex-wrap gap-1.5">
                        <button
                          v-for="preset in brandPresets"
                          :key="preset.id"
                          type="button"
                          class="size-5 cursor-pointer rounded-full border-2 transition-transform hover:scale-125"
                          :class="
                            preset.color === brandColor
                              ? 'border-(--ui-fg-strong)'
                              : 'border-transparent'
                          "
                          :style="{ background: preset.color }"
                          :title="`${preset.sect} · ${preset.name}`"
                          :aria-label="`换成${preset.sect}${preset.name}`"
                          :aria-pressed="preset.color === brandColor"
                          @click="selectBrand(preset.color)"
                        />
                      </div>
                    </div>
                  </div>
                </div>
              </XReveal>
            </li>
          </ul>
        </div>
      </section>

      <section id="rumors" class="scroll-mt-16 overflow-hidden">
        <div class="mx-auto max-w-5xl px-4 py-16 sm:px-6">
          <XReveal>
            <h2 class="text-2xl font-semibold text-(--ui-fg-strong)">门派频道</h2>
          </XReveal>
          <p class="mt-2 text-sm text-(--ui-fg-muted)">骚话谷出品，必属精品。</p>
          <ul class="mt-10 flex flex-col">
            <li
              v-for="(rumor, index) in rumors"
              :key="rumor.text"
              class="rumor-card w-[90%] px-6 py-7 sm:w-[70%] md:w-[55%]"
              :class="{ '-mt-3 sm:-mt-6': index > 0 }"
              :style="{
                '--i': index,
                '--z': index,
                '--tilt': `${index % 2 ? 1.5 : -1.5}deg`,
                '--offset': rumor.offset,
                '--offset-sm': rumor.offsetSm,
              }"
            >
              <span
                aria-hidden="true"
                class="absolute -top-3 start-4 font-serif text-5xl leading-none text-(--ui-primary)"
                >“</span
              >
              <div class="flex flex-wrap items-baseline gap-x-4 text-(--ui-fg-strong)">
                <span>{{ rumor.text }}</span>
                <span v-if="rumor.author" class="ms-auto">{{ rumor.author }}</span>
              </div>
            </li>
          </ul>
        </div>
      </section>

      <section>
        <div class="mx-auto max-w-5xl px-4 py-16 sm:px-6">
          <XReveal>
            <h2 class="text-2xl font-semibold text-(--ui-fg-strong)">用着放心</h2>
          </XReveal>
          <p class="mt-2 text-sm text-(--ui-fg-muted)">开源免费，安全无毒。</p>
          <ul class="mt-8 grid gap-4 sm:grid-cols-2">
            <li v-for="(item, index) in assurances" :key="item.title">
              <XReveal :delay="stagger(index)" class="h-full">
                <div class="ui-glow-card flex h-full gap-3 p-5">
                  <XIcon name="ph:check" class="mt-0.5 size-5 shrink-0 text-(--ui-success)" />
                  <div>
                    <h3 class="font-semibold text-(--ui-fg-strong)">{{ item.title }}</h3>
                    <p class="mt-1 text-sm text-(--ui-fg-muted)">{{ item.text }}</p>
                  </div>
                </div>
              </XReveal>
            </li>
          </ul>
          <div
          class="mt-10 rounded-(--ui-radius) border border-(--ui-warning) bg-(--ui-warning)/10 p-5 text-sm"
        >
          <p class="font-semibold text-(--ui-fg-strong)">使用前请知悉</p>
          <ul class="mt-2 list-disc space-y-1.5 ps-5 text-(--ui-fg)">
            <li v-for="caution in cautions" :key="caution">{{ caution }}</li>
          </ul>
        </div>
        </div>
      </section>

      <section id="changelog" class="scroll-mt-16 mx-auto max-w-5xl px-4 py-16 sm:px-6">
        <XReveal>
          <h2 class="text-2xl font-semibold text-(--ui-fg-strong)">更新公告</h2>
        </XReveal>
        <p class="mt-2 text-sm text-(--ui-fg-muted)">花花不骗花花，上面说的是真的！</p>
        <p v-if="!releases.length && !unavailable" class="mt-6 text-sm text-(--ui-fg-muted)">正在读取…</p>
        <p v-else-if="unavailable" class="mt-6 text-sm text-(--ui-fg-muted)">
          暂时读不到更新公告，可以到
          <a
            :href="`${REPOSITORY}/releases`"
            target="_blank"
            rel="noopener"
            class="text-(--ui-primary) underline-offset-2 hover:underline"
            >GitHub 发布页</a
          >查看。
        </p>
        <div v-else class="mt-6 space-y-3">
          <details
            v-for="(release, index) in releases"
            :key="release.tag"
            :open="index === 0"
            class="group rounded-(--ui-radius) border border-(--ui-primary)/30 bg-(--ui-primary)/5 transition-colors open:border-(--ui-primary)/60 hover:border-(--ui-primary)/60"
          >
            <summary
              class="flex cursor-pointer list-none items-center gap-3 px-4 py-3 select-none [&::-webkit-details-marker]:hidden"
            >
              <XIcon
                name="ph:caret-right"
                class="size-4 text-(--ui-fg-muted) transition-transform group-open:rotate-90"
              />
              <span class="font-semibold text-(--ui-fg-strong)">{{ release.tag }}</span>
              <XBadge v-if="release.tag === latest?.tag" tone="primary">最新</XBadge>
              <span class="ms-auto text-xs text-(--ui-fg-muted)">{{
                formatDate(release.publishedAt)
              }}</span>
            </summary>
            <div class="border-t border-(--ui-primary)/20 px-4 py-4">
              <ReleaseNotes :source="release.notes" />
              <p class="mt-4 flex flex-wrap gap-x-4 gap-y-1 text-xs text-(--ui-fg-muted)">
                <a
                  v-for="asset in release.assets"
                  :key="asset.name"
                  :href="asset.url"
                  class="hover:text-(--ui-fg)"
                  >{{ asset.kind === 'nsis' ? 'EXE' : 'MSI' }} · {{ formatSize(asset.size) }}</a
                >
                <a
                  :href="release.htmlUrl"
                  target="_blank"
                  rel="noopener"
                  class="hover:text-(--ui-fg)"
                  >GitHub 发布页</a
                >
              </p>
            </div>
          </details>
        </div>
      </section>

      <section id="support" class="scroll-mt-16">
        <div class="mx-auto max-w-5xl px-4 py-16 sm:px-6">
          <XReveal>
            <h2 class="text-2xl font-semibold text-(--ui-fg-strong)">支持&帮助</h2>
          </XReveal>
          <p class="mt-2 text-sm text-(--ui-fg-muted)">期望功能、优化建议、问题报告，欢迎骚扰，邮箱 1041367524@qq.com</p>
          <div class="mt-8 grid gap-4 sm:grid-cols-3">
            <XReveal :delay="stagger(0)" class="ui-glow-card flex flex-col p-5">
              <h3 class="font-semibold text-(--ui-fg-strong)">交流反馈</h3>
              <p class="mt-2 text-sm text-(--ui-fg-muted)">
                交流用法、分享配置、反馈问题，欢迎加入产品QQ群。
              </p>
              <p class="mt-3 text-sm text-(--ui-fg)">
                QQ群：<strong class="select-text font-semibold tabular-nums">{{ PRODUCT_QQ_GROUP }}</strong>
              </p>
              <button
                type="button"
                class="mt-3 inline-flex cursor-pointer items-center gap-1.5 self-start rounded-(--ui-radius) border border-(--ui-primary)/30 px-3 py-2 text-sm text-(--ui-primary) hover:bg-(--ui-primary)/10 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-(--ui-primary)"
                @click="copyGroupNumber"
              >
                <XIcon name="ph:copy" class="size-4" />
                复制群号
              </button>
              <p role="status" class="mt-2 text-xs text-(--ui-fg-muted)">{{ groupCopyStatus }}</p>
              <p class="mt-3 text-sm text-(--ui-fg-muted)">
                反馈异常时可附上诊断报告或日志。应用里 ☰ →「关于」也能复制群号。
              </p>
              <a
                :href="`${REPOSITORY}/issues`"
                target="_blank"
                rel="noopener"
                class="mt-auto inline-flex items-center gap-1.5 self-start pt-4 text-sm text-(--ui-primary) hover:underline"
              >
                前往 Issues
                <XIcon name="ph:arrow-up-right" class="size-4" />
              </a>
            </XReveal>
            <XReveal :delay="stagger(1)" class="ui-glow-card flex flex-col p-5">
              <h3 class="font-semibold text-(--ui-fg-strong)">常见问题</h3>
              <p class="mt-2 text-sm text-(--ui-fg-muted)">
                剑三里没反应、连太快收不住、旧配置兼容这些问题，使用说明书里都有解答。
              </p>
              <a
                :href="`${REPOSITORY}#用起来需要知道的几件事`"
                target="_blank"
                rel="noopener"
                class="mt-auto inline-flex items-center gap-1.5 self-start pt-4 text-sm text-(--ui-primary) hover:underline"
              >
                查看常见问题
                <XIcon name="ph:arrow-up-right" class="size-4" />
              </a>
            </XReveal>
            <XReveal :delay="stagger(2)" class="ui-glow-card flex flex-col p-5">
              <h3 class="font-semibold text-(--ui-fg-strong)">作者是谁</h3>
              <p class="mt-2 text-sm text-(--ui-fg-muted)">
                白天写编辑器、Agent 与基建，晚上给自己做打本小工具。想看还折腾了些什么，去个人站转转。
              </p>
              <div class="mt-auto flex flex-wrap items-center gap-x-4 gap-y-2 pt-4 text-sm">
                <a
                  :href="HOMEPAGE"
                  target="_blank"
                  rel="noopener"
                  class="inline-flex items-center gap-1.5 text-(--ui-primary) hover:underline"
                >
                  前往个人站
                  <XIcon name="ph:arrow-up-right" class="size-4" />
                </a>
                <!-- 产品索引页与本站同域（本站是它下面的一个子路径），走相对地址，预览环境也不会跳回线上 -->
                <a href="/" class="inline-flex items-center gap-1.5 text-(--ui-primary) hover:underline">
                  更多产品
                  <XIcon name="ph:arrow-up-right" class="size-4" />
                </a>
              </div>
            </XReveal>
          </div>
        </div>
      </section>
    </main>

    <!-- 右侧细轨：进度 + 章节刻度即页内导航。
         暂时整条藏起来（要放出来就把 class 换回 hidden xl:block——lg 下轨会压在卡片上）：
         库按「占住视口中部」判当前章节，本页首屏与最后一节都比视口矮，占不到中部，
         高亮会指错邻节；等 @xwink/ui 补上首尾兜底再开。
         锚点纠偏与清 hash 由页面的 useHashAnchor 做，轨上那份关掉，免得两份一起跟 -->
    <XScrollRail
      :sections="railSections"
      :align-hash="false"
      :clear-hash="false"
      class="hidden"
    />

    <footer class="border-t border-(--ui-border-muted)">
      <div
        class="mx-auto flex max-w-5xl flex-wrap items-center gap-x-4 gap-y-2 px-4 py-6 text-xs text-(--ui-fg-muted) sm:px-6"
      >
        <span>© 2026 x-wink</span>
        <a
          :href="`${REPOSITORY}/blob/main/LICENSE`"
          target="_blank"
          rel="noopener"
          class="hover:text-(--ui-fg)"
          >CC BY-NC-SA 4.0</a
        >
        <a
          :href="`${REPOSITORY}/blob/main/THIRD_PARTY.md`"
          target="_blank"
          rel="noopener"
          class="hover:text-(--ui-fg)"
          >第三方声明</a
        >
      </div>
    </footer>
  </div>
  </div>
</template>
