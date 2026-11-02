import type { TourDef, TourSnapshot } from '../types';
import { p } from './helpers';

function hasHoldRule(snapshot: TourSnapshot): boolean {
  return snapshot.rules.some((r) => r.mode === 'hold');
}

export const rules: TourDef = {
  id: 'rules',
  title: '规则玩法',
  summary: '筛选、规则卡、按键队列与录制、高级设置',
  steps: [
    {
      id: 'filter',
      target: 'filter',
      title: '一个列表，顶部只管筛选',
      body: p(
        '长按和切换规则都在同一个列表里，顶部的标签只筛选显示，不会删改规则。',
        '标签上的数字是「已启用 / 总数」。长按适合平 A、等 CD 戳键；切换适合挂机、采集、一键宏。',
      ),
      prepare: async (host) => {
        host.closeMenus();
        await host.setLayout('vertical');
        host.setFilter('all');
      },
    },
    {
      id: 'add',
      target: 'add-hold',
      title: '先加一条规则',
      body: p('后面几步要在规则卡上讲，先点这里添加一条长按连发规则。'),
      // 前置步：已经有长按规则就不出现，没有就等用户加一条，后面几步才有卡可指
      prepare: (host) => {
        host.setFilter('hold');
        return hasHoldRule(host.snapshot) ? 'skip' : undefined;
      },
      done: (now, entered) => now.rules.length > entered.rules.length,
      actionHint: '等你点一下…',
    },
    {
      id: 'card',
      target: 'rule-latest',
      title: '规则卡上有什么',
      body: p(
        '左边色条实心的是长按、描边的是切换；连发按键、间隔毫秒数、启用开关，头部中间的把手能拖动排序。',
        '正在连发时卡片会描边、呼吸高亮；被同组插队暂停时变淡并停止呼吸。',
        '多条规则一起连时软件会自动控速，不会越连越快、停不下来。',
      ),
      // 上一步被跳过、仍然没有规则卡：跳过而不是指着空处讲
      prepare: (host) => {
        host.setFilter('hold');
        return hasHoldRule(host.snapshot) ? undefined : 'skip';
      },
    },
    {
      id: 'mode',
      target: 'rule-mode',
      title: '点标签就能换模式',
      body: p(
        '卡片左上角的「长按 / 切换」标签点一下就换成另一种，不用删了重建。',
        '右上角的下拉箭头里还有移入分组、删除。',
      ),
      prepare: (host) => {
        host.setFilter('hold');
        return hasHoldRule(host.snapshot) ? undefined : 'skip';
      },
    },
    {
      id: 'queue',
      target: 'rule-latest',
      title: '单键也是一步队列',
      body: p(
        '每条规则能输出一份按键队列，每步有自己的间隔；长按／切换仍决定什么时候运行。',
        '「添加步骤／录制队列」里可以编辑、排序、录制和试播；保存成功才关闭编辑器，失败时保留草稿，取消保留原内容。',
        '录制支持普通键盘点按，倒计时 3 秒后开始。全局停止热键在全局关闭时也能结束录制；结束后先预览、设置末步到首步的间隔，再采用并保存。',
        '正常停止后从头开始；同组临时插队会保留步骤和剩余等待，发送失败的队列不会自动重放。',
        '停止或保存后仍按住的键需松开再触发。释放失败时同键队列等候重试；若提示输出线程异常，请重启应用。',
      ),
      prepare: (host) => {
        host.setFilter('hold');
        return hasHoldRule(host.snapshot) ? undefined : 'skip';
      },
    },
    {
      id: 'advanced',
      target: 'rule-advanced',
      title: '高级设置：两个键分开',
      body: p(
        '展开它可以把「启动键」和「连发键」分开：按住侧键连左键、按 F 连滚轮都行。',
        '切换连发还能再单独设一个停止键。',
        'DD 驱动允许启动、停止与连发用同一个键；同键启停异常时可关闭全局开关或正常退出应用，停止连发并释放按键。',
      ),
      prepare: (host) => {
        host.setFilter('hold');
        return hasHoldRule(host.snapshot) ? undefined : 'skip';
      },
    },
    {
      id: 'add-toggle',
      target: 'add-toggle',
      title: '添加切换连发',
      body: p(
        '切换连发规则从这里加；这一步只看，不用点。',
        '多段宏要互不打架，靠互斥组——另有一组教程「互斥组与多段宏」专门讲。',
      ),
      prepare: (host) => host.setFilter('all'),
    },
    {
      id: 'finish',
      title: '按键框变色是在提醒你',
      body: p(
        '规则的按键框描边变红或变黄是冲突提醒：热键盖住了触发键、这条的连发键是另一条的启动键之类。',
        '红的会让规则失效，黄的只是可能互相干扰，换个键就恢复。',
      ),
    },
  ],
};
