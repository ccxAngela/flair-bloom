// 产品页与打印排版（海报、说明书）共用的文案；只在一处改，两边不会说成两套话

export const REPOSITORY = 'https://github.com/x-wink/flair-bloom';
export { PRODUCT_QQ_GROUP } from '../../../main/src/constants';

// 作者个人站首页；app.xwink.fun 是产品索引页，页脚「更多产品」指那儿，两者不是一回事
export const HOMEPAGE = 'https://xwink.fun';

export type HighlightIconName = 'lightning' | 'palette' | 'keyboard' | 'gear' | 'float' | 'modes';

export interface TitledText {
  title: string;
  text: string;
}

// 亮点卡一张讲一批事，每条要点单列一行——挤成一整段话，读的人一条都记不住
export interface Highlight {
  icon: HighlightIconName;
  title: string;
  points: string[];
}

// 玩家群里反复被夸的几点，单独放大，不做评价墙也不署名。
// 顺序即版面顺序：错落网格按这个次序排，换位置要连带改 HIGHLIGHT_SPANS
export const highlights: Highlight[] = [
  {
    icon: 'palette',
    title: '颜值能打，外观随心配',
    points: ['追随光明，坠入黑暗', '20 种门派主题色任你挑'],
  },
  {
    icon: 'gear',
    title: '配置灵活可定制',
    points: ['键盘鼠标按键全支持', '提示语音随心所欲', '连发规则为所欲为', '多套规则一键切换'],
  },
  {
    icon: 'float',
    title: '多形态布局',
    points: ['竖版界面·最灵活', '横版界面·最直观', '悬浮窗口·超薄无感', '任务栏托盘·快捷入口'],
  },
  {
    icon: 'modes',
    title: '三模式覆盖',
    points: [
      '长按连发·手搓循环不浪费每一个GCD，挤挤总是会有的',
      '切换连发·一键宏、武学助手启动！DPS面板关闭！',
      '规则分组·多段宏无缝切换，长按插队松手复位',
    ],
  },
];

// 应用内的新手教程，页面上排成一条步骤轴
export const tourGuide: TitledText = {
  title: '萌新别懵逼，这里有教学',
  text: '教程指哪打哪，忘记了还可以再来一遍！',
};

export const tourSteps: TitledText[] = [
  { title: '初入江湖', text: '安装完就自动开始「上手三步」教学，不用抠脑壳。' },
  { title: '江湖秘籍', text: '各种用法手把手教学，学不会不要钱，学会了也不要钱。' },
  { title: '温故知新', text: '多主题教程打开右上角主菜单随时重新学习。' },
];

// 前四条是作者写的游戏梗，后两条摘自玩家群聊（不署名）；区块标题写明「骚话」，不冒充实名评价。
// offset 是桌面端左缩进，offsetSm 是手机端，错开摆放拼出互相压边的效果。
// 署名单独一个字段：混在正文里就得靠标签排版，页面和打印两处都得 v-html 才印得对
export interface Rumor {
  text: string;
  /** 有署名的那几条，落在同一行的右端 */
  author?: string;
  offset: string;
  offsetSm: string;
}

export const rumors: Rumor[] = [
  { text: '没用气质花之前过的都是什么苦日子啊 TAT', offset: '0%', offsetSm: '0%' },
  { text: '妹妹说的对！还是紫色最有韵味～', author: '--许嵩', offset: '42%', offsetSm: '10%' },
  { text: '花间游悟，你终于舍得把焚诀交出来了！', offset: '18%', offsetSm: '4%' },
  { text: '我劝你们别在测试服用，会影响正式服强度', offset: '4%', offsetSm: '0%' },
  { text: 'Alt键能绑定了，鼠标滚轮和侧键也能连发了，好耶！~', offset: '38%', offsetSm: '10%' },
  { text: '遥遥领先', author: '--玄武 meta1024', offset: '14%', offsetSm: '4%' },
];

export const cautions: string[] = [
  '它靠模拟按键工作，存在被游戏反作弊检测的风险；能不能用、会不会处罚，请自行评估承担。',
  '游戏里请用「游戏模式」，它会装一个小驱动并以管理员运行；个别游戏会拦它，遇到就切回「通用模式」。',
  'Windows 弹出「已保护你的电脑」时，点「更多信息 → 仍要运行」即可。',
];

// 取自 README 常见问题；封号风险与 SmartScreen 已在 cautions 里，这里不重复
export const faqs: TitledText[] = [
  {
    title: '剑三里没反应？',
    text: '先确认右下角是「全局已启用」、规则已勾选；剑三必须用「游戏模式」，按提示授权后重启。',
  },
  {
    title: '连太快、松手了还在按？',
    text: '间隔别填太小，建议 50ms 左右；多条一起连，软件会自动控速，不会停不下来。',
  },
  {
    title: '多段宏和长按能一起用吗？',
    text: '能，放进同一个互斥组：按住长按时宏让位，松手自己接着跑。',
  },
  { title: '旧配置还能用吗？', text: '能，覆盖安装直接兼容。' },
];

export const assurances: TitledText[] = [
  { title: '松手就停', text: '多条规则一起连会自动控总速，不会越叠越快、停不下来。' },
  { title: '不动游戏文件', text: '只是替你按键，不改游戏、不读游戏数据。' },
  {
    title: '配置存在自己电脑',
    text: '不用注册，按键与配置正文不上传。默认开启可关闭的最小使用统计，以环境标识关联使用记录；启动读取官网信息，激活时提交兑换码。',
  },
  { title: '更新省心', text: '新版本自动更新，国内下载也快，安装包都校验过防掉包。' },
];
