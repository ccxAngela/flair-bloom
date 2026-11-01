import type { TourDef } from '../types';
import { lastRule, p } from './helpers';

export const gettingStarted: TourDef = {
  id: 'getting-started',
  title: '上手三步',
  summary: '添加第一条规则并让它跑起来',
  steps: [
    {
      id: 'modes',
      target: 'add-buttons',
      title: '两种连发',
      body: p(
        '长按连发：按住那个键就一直连，松手就停。',
        '切换连发：按一下开始连，再按一下停。先从长按连发开始。',
        '已有 .qzh 或其他助手配置可从「☰ → 导入配置」迁移，先扫描和预览，再由你确认。',
      ),
      // 横版键鼠图没有筛选条，也没有规则卡，整组都讲不了
      prepare: async (host) => {
        host.closeMenus();
        await host.setLayout('vertical');
        host.setFilter('all');
      },
    },
    {
      id: 'add',
      target: 'add-hold',
      title: '添加第一条规则',
      body: p('点这个按钮，下面会出现一张规则卡。'),
      prepare: (host) => host.setFilter('all'),
      done: (now, entered) => now.rules.length > entered.rules.length,
      actionHint: '等你点一下…',
    },
    {
      id: 'key',
      target: 'rule-key',
      title: '按下要连的键',
      body: p(
        '点一下这个按键框，再按键盘或鼠标上要连的那个键。',
        '鼠标左右中键、两个侧键、滚轮上下都能录。',
      ),
      // 上一步「添加规则」被跳过且列表仍为空：这一步和下一步没有卡可指，直接跳过
      prepare: (host) => (host.snapshot.rules.length === 0 ? 'skip' : undefined),
      // 按「录入过一次」判定而不是比较键值：新规则的连发按键默认就是 Q，
      // 照提示按 Q 的用户键值不变，会卡在这一步出不去。
      done: (now, entered) => now.keyCaptureSeq > entered.keyCaptureSeq,
      actionHint: '等你按一个键…',
    },
    {
      id: 'enable',
      target: 'rule-enable',
      title: '启用这条规则',
      body: p(
        '勾上这个开关，这条规则才会跑。',
        '旁边的间隔是每次按键相隔多少毫秒，默认值一般够用，调得太小软件会自动控速。',
      ),
      prepare: (host) => (host.snapshot.rules.length === 0 ? 'skip' : undefined),
      // 让用户动手的步骤必须是实操步：讲解步会铺一层 blocker，高亮的开关也点不动
      done: (now) => lastRule(now)?.enabled === true,
      actionHint: '等你勾上它…',
    },
    {
      id: 'global',
      target: 'global',
      title: '打开总开关',
      body: p(
        '点右下角「全局已禁用」，让它变成「全局已启用」，背景变色就生效了。',
        '按住刚才录的那个键就开始自动连发，松手停。',
      ),
      done: (now) => now.globalEnabled,
      actionHint: '等你点亮它…',
    },
    {
      id: 'finish',
      title: '会了',
      body: p(
        '☰ 菜单里的「新手教程」还有六组，随时能回来看。',
        '要在游戏里用，接着看「游戏模式与驱动」那一组。',
      ),
    },
  ],
};
