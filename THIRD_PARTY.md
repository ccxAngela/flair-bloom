# 第三方组件声明

## Windows / WebView2 Rust 接口绑定

- 项目：[windows-rs](https://github.com/microsoft/windows-rs)（`windows` 0.61，MIT OR Apache-2.0）、[webview2-rs](https://github.com/wravery/webview2-rs)（`webview2-com` 0.38，MIT）。
- 用途：读取 WebView2 实际缩放与控制器尺寸，设置显示器 DPI 缩放以避免系统文字大小裁切界面。
- 集成方式：仅 Windows 目标静态链接原版 Rust 接口绑定；WebView2 Runtime 仍由系统提供。

## Interception Driver

- 项目：[Interception](https://github.com/oblitum/Interception)
- 作者：Francisco Lopes (oblitum)
- 许可证：LGPL-3.0（非商业用途）
- 用途：内核级键盘输入模拟，用于兼容使用 Raw Input API 的游戏（如剑网三）
- 集成方式：
  - `interception-sys` crate（v0.1.3）— 编译期静态链接用户态库
  - `install-interception.exe`（v1.0.1）— 打包为应用资源，供用户安装内核驱动

### 说明

Interception 驱动为可选组件，对应输入模式「游戏模式」。默认的「通用模式」（SendInput）不依赖此驱动。

## DD 虚拟键鼠 SDK

- 项目：[ddxoft/master](https://github.com/ddxoft/master)
- 作者：ddxoft
- 用途：虚拟键鼠驱动，作为「游戏模式」之外的备用输入通道，兼容部分游戏模式仍无效的程序
- 集成方式（按原作者发布的二进制原样打包，未做修改或重新签名）：
  - `apps/main/src-tauri/resources/dd63330.dll` — 「DD驱动」模式使用的 DD DLL
  - `apps/main/src-tauri/resources/ddhid-driver/` — DD-HID 驱动包，仅用于卸载存量安装：
    - `ddc.exe` — PnP 卸载工具
    - `ddhid63340.sys` / `ddhid63340.cat` / `ddhid63340.inf` — WHQL 签名的 HID-Class 驱动
- 运行时仅通过 `LoadLibraryW` + `GetProcAddress` 调用 DD SDK 导出函数：`DD_btn`、`DD_key`、`DD_todc`、`DD_whl`

### 说明

「DD驱动」模式使用 `dd63330.dll`，要求宿主进程以管理员身份运行。DD 驱动无法标识自身注入的按键，因此「目标键」不能与「触发键 / 停止键」重合于切换连发规则，本软件已在配置校验和模式切换处强制此约束。

DDHID 模式因稳定性风险（可能导致蓝屏）已永久移除，安装链路与用户态 DLL 均已删除。驱动包保留在安装目录仅为卸载存量安装：如系统中已安装 DDHID 驱动，请在应用「诊断修复」中卸载。

如对 DD 驱动的来源、签名或行为有疑问，请以原作者仓库说明为准。

## serialport-rs

- 项目：[serialport-rs](https://github.com/serialport/serialport-rs)
- 许可证：MPL-2.0
- 用途：「外设模式」经 USB 串口与外设驱动通信、按需枚举串口
- 集成方式：`serialport` crate（v4）原样作为依赖静态链接，未修改其源码；源码见上方仓库或 crates.io

## JX3Box 公开数据包

- 项目：[@jx3box/jx3box-data](https://www.npmjs.com/package/@jx3box/jx3box-data)（v3.9.9）
- 作者：JX3Box
- 许可证：MIT
- 用途：剑网三专栏的心法表（心法编号、名称、定位，取自 `xf.json` 与 `mount_group.json`）与无相楼主题色（`colors.json`）
- 集成方式：按其中的数据手工整理进 `apps/site/public/jx3/column.json` 与 `theme.ts`，不依赖、不打包这个包

### 说明

专栏里的白皮书与宏只收标题、作者与原文链接（作者授权时另收宏正文），版权归原作者，内容以原站为准。
