import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import {
  BRAND_STORAGE_KEY,
  brandPresets,
  brandVariables,
  defaultBrandColor,
} from './app/utils/brand.ts';

const baseURL = '/flair-bloom/';

const brandTable = Object.fromEntries(
  brandPresets.map((preset) => [preset.color, brandVariables(preset.color)]),
);

// SSG 首帧没有所选门派色，等客户端补上会先闪一帧；在样式生效前按已存选择打上。
// 门派色只认色板里的值，存储被改坏时回落默认色。明暗档不在这里：@xwink/ui/nuxt 按 xwink.fun
// 全站共享的主题 Cookie 接管首帧，与主站、产品索引页同一份偏好。
const brandBootstrap = `(function(){try{var d=document.documentElement,t=${JSON.stringify(brandTable)},v=t[localStorage.getItem(${JSON.stringify(BRAND_STORAGE_KEY)})]||t[${JSON.stringify(defaultBrandColor)}];for(var k in v)d.style.setProperty(k,v[k])}catch(e){}})()`;

export default defineNuxtConfig({
  compatibilityDate: '2026-01-01',
  runtimeConfig: {
    public: {
      clientBootstrapUrl: 'https://platform.xwink.fun/api/v1/client-bootstrap/flair-bloom',
    },
  },
  devServer: { port: 3900 },
  modules: ['@xwink/ui/nuxt'],
  alias: {
    // 与应用内更新公告共用同一个解析器，网站与应用对公告语法的支持范围不会漂
    '#markdown-parse': fileURLToPath(
      new URL('../main/src/windows/panel/components/markdown-parse.ts', import.meta.url),
    ),
  },
  app: {
    baseURL,
    head: {
      htmlAttrs: { lang: 'zh-CN' },
      title: '气质花按键助手 FlairBloom · 下载与更新公告',
      meta: [
        {
          name: 'description',
          content:
            'PVE 打本按键小助手：长按连发、一键宏与多段宏切换。Windows 10 / 11 安装包国内直连下载，附完整更新公告。',
        },
        { property: 'og:title', content: '气质花按键助手 FlairBloom' },
        { property: 'og:description', content: 'PVE 打本按键小助手，让手指歇会儿。' },
        { property: 'og:image', content: 'https://app.xwink.fun/flair-bloom/icon.png' },
      ],
      link: [{ rel: 'icon', type: 'image/png', sizes: '32x32', href: `${baseURL}favicon-32.png` }],
      script: [{ innerHTML: brandBootstrap, tagPosition: 'head' }],
    },
  },
  typescript: {
    tsConfig: {
      compilerOptions: {
        // Nuxt 4 默认开启，主应用的 tsconfig 没开；共享的 markdown-parse 按主应用口径编写，
        // 为网站改主应用代码不值当，这里对齐主应用
        noUncheckedIndexedAccess: false,
      },
    },
  },
  css: ['~/assets/css/main.css'],
  nitro: {
    prerender: { crawlLinks: false, routes: ['/', '/download', '/privacy'] },
  },
  vite: {
    plugins: [tailwindcss()],
    server: {
      fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] },
    },
  },
});
