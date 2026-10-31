import type { TourDef } from '../types';
import { p } from './helpers';

export const settings: TourDef = {
  id: 'settings',
  title: '热键、声音与外观',
  summary: '全局热键、提示音与主题',
  steps: [
    {
      id: 'hotkeys',
      target: 'settings-hotkeys',
      title: '三个全局热键',
      body: p(
        '全局开关、停止键、面板显隐——打着游戏不用切出来就能操作。',
        '点按键框开始捕获，按下要绑的键；框上右键可以清除。',
      ),
      prepare: (host) => {
        host.closeMenus();
        host.openSettings('hotkeys');
      },
    },
    {
      id: 'hotkey-limits',
      target: 'settings-hotkeys',
      title: '热键的几条限制',
      body: p(
        '热键只能是键盘键，Shift / Ctrl / Alt / Win 也行，左右分开识别；三个热键不能重复。',
        '绑了右 Alt 又绑左 Ctrl 会互相牵连，这是键盘布局的老毛病，界面会提示。',
      ),
      prepare: (host) => host.openSettings('hotkeys'),
    },
    {
      id: 'sound',
      target: 'settings-sound',
      title: '声音播报',
      body: p(
        '全局启用、全局停用、切换连发开始、切换连发结束四个时机，各自能单独开关。',
        '每个时机可以选朗读一句话或放一段音频，音量、语速、音调都能调。',
      ),
      prepare: (host) => host.openSettings('sound'),
    },
    {
      id: 'theme',
      target: 'settings-general-theme',
      title: '换个颜色',
      body: p(
        '明亮 / 黑暗 / 跟随系统，再挑一个门派配色，一共 21 套。',
        '☰ 菜单 → 主题颜色也能快捷切换。',
        '屏幕大嫌字小，拖面板边缘就能等比放大，这里可以一键复位。',
        '界面不受 Windows 文字大小影响；这里复位应用缩放，不会修改系统设置。',
      ),
      prepare: (host) => host.openSettings('general'),
    },
    {
      id: 'startup',
      target: 'settings-general-startup',
      title: '启动与权限',
      body: p(
        '「启动后自动开全局」打开应用就开始连发；「以管理员模式启动」省掉进游戏模式时的提权重启。',
        '这两个都和「开机自启」互斥，开一个会自动关掉另一个——开机就连发容易误触发，而需要管理员权限的程序开机自启拉不起来。',
      ),
      prepare: (host) => host.openSettings('general'),
    },
    {
      id: 'general',
      target: 'settings-general-close',
      title: '关闭行为',
      body: p(
        '点 ✕ 时是退出还是收进浮窗，在这里选。最小化不用选：任何方式收起来都进浮窗，不会只剩托盘图标。',
        '自动更新关掉后只在标题栏提示新版本，菜单里的「检查更新」仍然会下载。',
      ),
      prepare: (host) => host.openSettings('general'),
    },
    {
      id: 'finish',
      title: '热键跟着配置走',
      body: p('全局热键存在当前配置里，换一套配置热键也会跟着换。'),
      // 教程是自己开的设置弹窗，走完顺手收掉，别把它留在屏幕上
      prepare: (host) => host.closeSettings(),
    },
  ],
};
