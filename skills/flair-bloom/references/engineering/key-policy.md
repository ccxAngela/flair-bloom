# 按键标识、角色与录入策略

## `KeyId`

按键标识是 tagged union，前后端共享 wire format `{kind:"keyboard",code:81}` / `{kind:"mouse",code:"left"}`。`MouseButton` 含 `Left/Right/Middle/X1/X2` + `WheelUp/WheelDown`（滚轮，瞬发）。规则的 `trigger_key`、`steps[].key`、`stop_key` 与三个全局热键都使用单个 `KeyId`，没有组合键绑定。`PENDING_INJECTIONS` 注入事件队列也以 `(KeyId, is_up)` 为键。定义在 `packages/qzh-profile/src/key_id.rs`，前端镜像在 `components/KeyCapture.tsx`。

## 角色模型（唯一事实源：`packages/qzh-profile/src/key_policy.rs`）

系统里的按键只有两种角色。**读**角色只被低级钩子观察、永不注入（三个全局热键、规则的 `trigger_key` 与 `stop_key`）；**写**角色经统一输入通道注入、不参与触发判定（每个 `steps[].key`）。校验与净化遍历整条队列的去重按键集合，重合关系检查任一步，不只看首步。驱动支持只对写角色有意义。

一个槽位可能同时承担两种角色（默认模式与横版单键模型的连发按键，`trigger == target`），此时受两套约束的**交集**。方向与权限模型的 `rw` 相反：权限枚举「被授予的动作」，角色越多动作越多故取并集；这里枚举「够格的按键」，角色越多要求越多故取交集。`KeySlot` 的五个变体就是这个模型：`Hotkey` / `Trigger` / `Target` / `TriggerTarget` / `TriggerTargetToggle`。重合态按规则模式再分一层，因为切换连发对自注入过滤的要求比长按高。

限制来自两个源，模块里分开表达。**能力**是后端做不做得到，随输入模式变，由 `InjectCaps` 从 `win-input` 的 `InputMode` 谓词填充（DD-HID 移除后只剩 `coincident_toggle` 一位；将来若有后端受限，加能力位即可，`SlotPolicy` 会自动跟着变）；**策略**是产品上让不让做（热键禁鼠标、规则禁修饰键），与输入模式无关，写死在模块里。二者都不是对方，混在一起模型会裂：热键槽和启动键槽同为纯读，但前者禁鼠标后者不禁。

DDSimple 的 `coincident_toggle` 为 true，允许 Toggle 的启动 / 停止键与连发键重合，也允许横版键鼠图。DD 的 `ExtraInformation` 被驱动写死为 0，自注入只能靠时间窗口队列过滤，对重合键无法可靠区分「用户按下」与「自身回灌」，Toggle / Hold 都可能误判。产品接受这份不可靠性，以全局关闭和正常退出应用时的停连发、抬键清理兜底；这不保证同键启停本身可靠，也不覆盖强杀进程。外设模式仍限制同键 Toggle，`requires_distinct_target_for_toggle` 仅对它返回 true，`TriggerTargetToggle` 是空集，键盘键也不收（`SlotPolicy.keyboard` 为 false）。判定写在 `accepts` 的开头，`slot_policy` 与 `find_rule_violations` 都由它派生。横版每个键位都是重合态，`switchLayout` 直接以 `coincident_toggle` 为准；切到目标模式前的界面拦截仅限制外设模式。外设模式全面禁用横版仍带一层产品简化（Hold 的重合态其实允许）。

## 判定与提醒分两层

本模块管**硬性正确性**：这个键放进这个槽位对不对，错了就拦。前端 `conflicts.ts` 管**软性提醒**：绑定之间可能互相干扰（面板键遮蔽全局键、热键盖住规则触发键、A 的连发键是 B 的触发键），是跨绑定关系，不可能由前者推出来，两者互不替代。

## 强制点

`set_rules` / `set_input_mode` / `set_global_hotkeys` 三个命令拒绝违规写入，`Profile::validate()` 只管规则数与间隔、**刻意不查按键**——否则含不支持按键的旧配置会整个打不开。所有配置装载都必须经 `activate_profile_file`（启动加载、托盘切换、`load_profile`、导入 `.qzh`、外部导入、删除激活配置后的回落，共六条路径），由它统一 `sanitize_profile` 净化后再 `set_rules` / `set_hotkeys`；命令里自己拼这段序列就会漏掉净化。

净化规则：热键**置为未绑定**（那本来就是它的合法状态），规则**只停用不动按键**（改写成空会造出新的中间态，连发索引、冲突检测、界面渲染都要加分支，连锁面太大）。结果存进 `ProfileNotice`，前端挂载时用 `take_profile_notice` 取走弹窗；启动加载先于窗口创建，直接发事件会丢，运行期切换配置才额外发 `profile-sanitized`。

## 前端录入

前端不自行维护白名单，`get_key_policy` 按当前输入模式下发五个槽位的允许集（`SlotPolicy { keyboard, modifiers, mouse[] }`），`KeyCapture` 只判断「在不在集合里」，不在就带原因回调（`modifier` / `unknown` / `mouse-unsupported` / `slot-disabled`）交给页面提示；`keyboard` 为 false 的槽位连普通键盘键都不收。规则卡片按 `rule.mode` 在 `trigger_target` 与 `trigger_target_toggle` 之间选。输入模式切换后需重新拉取（`switchLayout` 还会现拉一次，因为切模式后策略异步重取，缓存值可能是旧模式的）。
