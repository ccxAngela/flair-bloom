import type { TourDef } from '../types';
import { p } from './helpers';

export const layouts: TourDef = {
  id: 'layouts',
  title: '横版键鼠图与浮窗',
  summary: '在键鼠图上直接点键，最小化成浮窗',
  // 横版是单键模型（trigger == target），外设模式建不出切换连发，switchLayout 本来就会拦；
  // 教程不绕过这条互斥，在目录里就说明白。
  available: (snapshot) => (snapshot.coincidentToggle ? undefined : '当前输入模式不支持横版键鼠图'),
  steps: [
    {
      id: 'toggle',
      target: 'layout-toggle',
      title: '两种布局',
      body: p(
        '标题栏这个按钮在竖版规则列表和横版键鼠图之间切换。',
        '竖版适合精细编辑，横版一眼看清哪些键在用。',
        '界面跟随显示器缩放，不受 Windows 文字大小影响；需要放大时可以拖动面板边缘。',
        'DD 驱动可用横版；外设模式暂不支持横版。',
      ),
      // 从竖版起步，下一步切过去才有对照
      prepare: async (host) => {
        host.closeMenus();
        await host.setLayout('vertical');
      },
    },
    {
      id: 'click-key',
      target: 'hkb-keyboard',
      title: '直接点键设连发',
      body: p(
        '在键鼠图上点一个键：无 → 切换连发 → 长按连发 → 取消，三态轮着来。',
        '右键点它还有更多选项：启用、改模式、分组、删除。',
      ),
      prepare: (host) => host.setLayout('horizontal'),
    },
    {
      id: 'legend',
      target: 'hbar-legend',
      title: '颜色和角标的意思',
      body: p(
        '实心是长按连发、描边是切换连发、虚线是已停用；数字角标是互斥组的编号，实心描边同理。',
        '⚠ 表示这个键上有横版画不出来的规则（启动键和连发键不同、或同一个键上有多条），回竖版改；鼠标移上去会画出它和别的键的走线。',
      ),
      prepare: (host) => host.setLayout('horizontal'),
    },
    {
      id: 'interval',
      target: 'hbar-interval',
      title: '横版共用一个间隔',
      body: p('横版上点出来的规则都用这一个间隔，要分别设不同间隔请回竖版。'),
      prepare: (host) => host.setLayout('horizontal'),
    },
    {
      id: 'float',
      target: 'minimize',
      title: '收进浮窗',
      body: p(
        '点「—」把面板收成一个常驻置顶的小条，想放哪拖到哪，位置会记住。',
        '小条上能看到正在连发的键（会呼吸闪烁），也能直接启停、点开回主面板。',
      ),
      // 教程结束时布局停在竖版，别把用户丢在横版
      prepare: (host) => host.setLayout('vertical'),
    },
    {
      id: 'finish',
      title: '关窗口的行为也能改',
      body: p('点 ✕ 关窗口时是退出还是收进浮窗，在设置 → 通用里选。'),
    },
  ],
};
