import type { TourDef } from '../types';
import { p } from './helpers';

export const profiles: TourDef = {
  id: 'profiles',
  title: '多套配置',
  summary: '新建、切换与导入导出配置',
  steps: [
    {
      id: 'current',
      target: 'profile',
      title: '当前用的是哪套',
      body: p(
        '左下角显示当前配置。一个角色一套连发键位，换角色直接换配置。',
        '改动「默认配置」时会自动另存成一套新的，默认配置本身不会被改坏。',
      ),
      prepare: async (host) => {
        host.closeMenus();
        await host.setLayout('vertical');
      },
    },
    {
      id: 'switch',
      target: 'profile',
      title: '切换和新建',
      body: p(
        '点开它就能切换已有配置、新建一套，或者进去管理。',
        '切换、导入和管理前会先保存当前编辑；保存失败会提示并中止本次操作，可继续编辑后重试。',
      ),
    },
    {
      id: 'manage',
      target: 'settings-tabs',
      title: '在设置里管理',
      body: p(
        '设置 → 配置文件：重命名、删除、导出；「导入配置」统一导入 .qzh，以及丐帮高手、衍天高手、极简按键和小叽按键配置。',
        '扫描默认位置及已知工具的桌面快捷方式目录，管理目录之外的 .qzh 也能发现，多档方案分别列出；也可选择文件或指定目录。',
        '导入前查看预览：不能等价转换的动作会说明并跳过，独立按压时长也会提示差异。',
        '主菜单也有「导入配置」，不必先进设置。',
      ),
      prepare: (host) => host.openSettings('profiles'),
    },
    {
      id: 'tray',
      title: '游戏里不开面板也能换',
      body: p('托盘图标右键有「切换配置」，打着游戏也能直接换一套。'),
    },
    {
      id: 'finish',
      title: '分享给朋友',
      // 教程是自己开的设置弹窗，走完顺手收掉，别把它留在屏幕上
      prepare: (host) => host.closeSettings(),
      body: p(
        '导出的 .qzh 发给朋友，对方在「导入配置」中选择文件、预览并确认即可。',
        '全局热键也存在配置里，换配置会跟着一起换。欢迎自行尝试解析 .qzh，开发兼容转换工具。',
      ),
    },
  ],
};
