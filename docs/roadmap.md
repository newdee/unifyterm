# 路线图

## 本期（M0–M7）

- 已验收：M0、M1、M2、M3、M4、M5、M6、M7。每轮验收的数字见 `docs/verification-log.md`。
- 已发布：**0.1.0**，2026-11-18，标签 `v0.1.0`（提交 `b93e5ff`）。Release 里有 9 个安装文件，各带一个 `.sha256`。scoop 清单已由工作流提交回 `main`；formula 在 `newdee/homebrew-tap`；网页在 https://dfine.tech/unifyterm/ 。
- 已发布：**0.1.1**，2026-11-19，标签 `v0.1.1`（提交 `7b3962b`）：macOS 上从桌面启动的三个问题（登录 shell、标签页从主目录开始、侧栏用登录 shell 找 keepane）。scoop 清单和 formula 都已是 0.1.1。三处修复已在一台真实的 Mac 上确认（见验收记录）。
- 已发布：**0.2.0**，2026-11-19，标签 `v0.2.0`（提交 `ca11002`）：程序文件带图标；Windows 和 macOS 上标签栏兼作标题栏；`ut setting` 设置界面，窗口随设置文件的变化立即更新。macOS 上去掉标题栏后的样子还没有人看过。
- 已发布：**0.2.1**、**0.2.2**，2026-11-19：设置界面里选择（0.2.1）；Windows 程序自带 C 运行库（0.2.2）。
- 已发布：**0.2.3**，2026-11-20，标签 `v0.2.3`（提交 `09f6c56`）：`ut shell enable`（所有 shell 里用 ut 的命令，含 keepane 窗格）、全屏（F11、Alt+Enter；macOS Cmd+Enter）、`updates.check` 更新检测、Windows 上键盘被隐藏的控制台窗口拿走时收回、MSI 默认建桌面图标且快捷方式显示程序图标。Release 18 个文件；formula 和 Scoop bucket 都是 0.2.3，sha256 和 Release 一致。
- 已发布：**0.2.4**，2026-11-20，标签 `v0.2.4`（提交 `019e25b`）：skill 写明命令在哪里能用、agent 的 PowerShell 里写 `cat.exe`；`find -name`、`grep --include` 这类选项的值不展开（agent 的 `find . -name "*.md"` 原来会坏）；标签页不再继承上一层 UnifyTerm 的 `UNIFYTERM_*` 变量。Release 18 个文件；formula 和 Scoop bucket 都是 0.2.4，4 个哈希和 Release 一致。
- 已发布：**0.3.0**，2026-11-24，标签 `v0.3.0`（提交 `03c918e`）：行首 `#` 加一句话，回车后换成模型写的命令（默认不运行）；模型来源新增 OpenAI、Ollama、LM Studio、vLLM 和本机的 agent 命令行（Claude Code、Codex、pi、任意程序）；macOS 上长按字母键重复输入；第一次同意后"解释"的等待框不再卡住。Release 18 个文件；formula 和 Scoop bucket 都是 0.3.0，4 个哈希和 Release 一致。macOS 上的长按和从程序坞启动时找 agent 命令行，还没有人在真实的 Mac 上确认。
- 还没做、要用户来做的发布步骤：签名（Windows、macOS）、提交到 winget、在干净的机器上各装一次，见 `docs/release.md`。
- 2026-11-18 改名为 UnifyTerm（原名 uni-term），加上 brew、scoop、curl 安装方式和宣传视频。
- 未做：无。
- 方案总览见 `demo/index.html` 的"方案"页。

## 下一步（2026-11-23 定）

1. **M8：用一句话写命令；更多的模型来源。** 已做完，随 0.3.0 发布（2026-11-24）。设计和实现时改动的决定在 `docs/plan-m8.md`。行首 `#` 加一句话，回车后换成命令；来源增加 OpenAI、Ollama、LM Studio、vLLM 的预设，和本机的 agent 命令行（Claude Code、Codex、pi、任意命令）。不带模型，不下载模型。还没有用真实的 Ollama、LM Studio、vLLM、OpenAI 试过。
2. **M9：命令清单协议。** UnifyTerm 一侧已做完（2026-11-24），在 `main` 上，还没有发版。设计和实现时改动的决定在 `docs/plan-m9.md`。程序用 OpenCLI 0.1（JSON）说明自己的命令、选项和参数；`ut program add` 存进数据目录的 `programs/`；窗口用它做四件事：输入行下面的列表、失败后的纠正、不用模型的解释（Ctrl-Shift-E）、给模型看并核对模型的回答。用 git、curl、ffmpeg、dotnet 的真实清单试过。keepane 一侧（打印自己的清单）已在群聊 `#unifyterm-keepane` 里提出，还没有做。
3. **M10：主题**（用户 2026-11-24 提出）：自带几套主题，允许自己写主题文件，一条命令切换，重开后保持。**只有设计**，在 `docs/plan-m10.md`；用户说决定做了再开始写代码。

## 自己的 shell：另起一个项目 UnifyShell

用户 2026-11-18 决定：自己的 shell 不放在本仓库里做，单独开一个项目。

- 项目名和包名：`unifyshell`（GitHub 和 crates.io 上都没人用过，2026-11-18 查）。给人看写 UnifyShell。
- 命令名：`ush`。本机和 WSL 里都不是已有命令。注意 crates.io 上 `ush` 这个包名已被占（"Ultrasonic Shell"），所以包名不能叫 `ush`，只有装出来的程序叫 `ush`，与 UnifyTerm 配 `ut` 的做法相同。
- 为什么分开：shell 的工作量大（语法、作业控制、补全、三个平台的进程模型），发布节奏和终端不同；别的终端也能用它；UnifyTerm 只需要把它当作又一种可托管的 shell（和 PowerShell、bash 并列），两边通过 OSC 133/633 标记对接，不需要共享代码。
- 目的（用户 2026-11-17 定）：三个平台用同一种 shell 语法；shell 这一层的路径全部是 `/`。
- 两条路线，开项目时再选：嵌入 brush，或从零写。
- 现状：还没开始，也还没有计划。`docs/plan.md` 末尾保留着当初的调研结果（brush 等），开项目时搬过去。
- 在那之前，UnifyTerm 只托管系统 shell（PowerShell、cmd、bash、zsh、fish）。M2 为 PowerShell 做的补丁（别名移除、提示符、引号标记）继续保留。
## 下一期（通信功能、可观测平台）：不做

用户 2026-11-17 提出过这两项；2026-11-18 决定不做，原话："下一期先不做"。

原因：与 keepane 重叠。逐项对照过 keepane 现有的命令：

| 设想的功能 | keepane 已有的 |
|---|---|
| 人与 agent、agent 与 agent 之间发消息 | `send-message`、`read-message`、`list-messages`、`trace-message`，每个窗格有收件箱 |
| 两台机器之间 | `link add/share/watch/attach/mirror`，`send-message --to host:port/%name` |
| 手机上看和发、传文件、通知 | `web`、`share`、`notify` |
| agent 的模型、费用、token、上下文、CPU、内存 | `list-agents -J`、`agent-cost` |
| agent 一步步做了什么、事件、任务 | `trace-pane`、`list-events`、`list-tasks` |
| 总览界面 | `dashboard`、手机页面 |

分工：keepane 管多窗格、agent、通信、可观测；UnifyTerm 只做跨平台 GUI 终端。

- 现有的 agent 侧栏（Ctrl-Shift-A）保留，但不在"把 keepane 的数据再画一遍"这个方向上继续加功能；想加的功能加到 keepane 里。
- 可选的小功能，不排期：`ut stats`（按目录或项目统计命令的耗时和失败率）。这是 UnifyTerm 历史库独有的数据，keepane 没有。