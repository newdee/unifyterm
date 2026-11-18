# 路线图

## 本期（M0–M7）

- 已验收：M0、M1、M2、M3、M4、M5、M6、M7。M7 的发布步骤（推送、CI、签名、提交到 winget 和 Homebrew）等用户确认，见 `docs/release.md`。每轮验收的数字见 `docs/verification-log.md`。
- 2026-11-18 改名为 UnifyTerm（原名 uni-term），加上 brew、scoop、curl 安装方式和宣传视频。
- 未做：无。
- 方案总览见 `demo/index.html` 的"方案"页。

## 自己的 shell：另起一个项目 UnifyShell

用户 2026-11-18 决定：自己的 shell 不放在本仓库里做，单独开一个项目。

- 项目名和包名：`unifyshell`（GitHub 和 crates.io 上都没人用过，2026-11-18 查）。给人看写 UnifyShell。
- 命令名：`ush`。本机和 WSL 里都不是已有命令。注意 crates.io 上 `ush` 这个包名已被占（"Ultrasonic Shell"），所以包名不能叫 `ush`，只有装出来的程序叫 `ush`，与 UnifyTerm 配 `ut` 的做法相同。
- 为什么分开：shell 的工作量大（语法、作业控制、补全、三个平台的进程模型），发布节奏和终端不同；别的终端也能用它；UnifyTerm 只需要把它当作又一种可托管的 shell（和 PowerShell、bash 并列），两边通过 OSC 133/633 标记对接，不需要共享代码。
- 目的（用户 2026-11-17 定）：三个平台用同一种 shell 语法；shell 这一层的路径全部是 `/`。
- 两条路线，开项目时再选：嵌入 brush，或从零写。
- 现状：还没开始，也还没有计划。`docs/plan.md` 末尾保留着当初的调研结果（brush 等），开项目时搬过去。
- 在那之前，UnifyTerm 只托管系统 shell（PowerShell、cmd、bash、zsh、fish）。M2 为 PowerShell 做的补丁（别名移除、提示符、引号标记）继续保留。
## 下一期（M7 完成后再规划）

用户 2026-11-17 提出，原话："我希望我们的 iterm 也需要具备通信功能，和可观测平台，这全部的目标实现后，作为下一期的规划一下。"

范围只有这两项，细节未定：

1. **通信功能**
2. **可观测平台**

规划时要先和用户确认的问题（现在不展开，不做假设）：

- 通信的对象是谁：标签页之间、UnifyTerm 窗口之间、两台机器之间，还是人与 agent 之间。
- 和 keepane 已有的收件箱、`keepane link`（跨机器）是什么关系：复用、在 GUI 里呈现，还是另做一套。
- 可观测的对象是什么：agent 的轮次与工具调用、命令的耗时与退出码、终端自身的性能，还是全部。
- 可观测平台是本机界面，还是可以多机汇总的服务。
- 数据留在本地，还是要导出到外部系统（例如 OpenTelemetry）。

本期里会给下一期留下可用基础的部分：M3 的命令历史库（每条命令的目录、退出码、耗时），M6 的 agent 侧栏（读取 keepane 的 agent 状态）。
