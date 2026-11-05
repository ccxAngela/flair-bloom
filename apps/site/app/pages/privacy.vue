<script setup lang="ts">
import agreement from '../../../main/src/assets/EULA.md?raw';
useSeoMeta({ title: '用户协议与数据政策 · 气质花' });
</script>
<template>
  <main class="mx-auto max-w-3xl px-6 py-16 text-(--ui-fg)">
    <NuxtLink to="/" class="text-(--ui-primary)">返回气质花官网</NuxtLink>
    <ReleaseNotes :source="agreement" class="mt-8" />
  </main>
</template>
