# 验收记录

## 2026-11-18 · 发布前的 CI 修复；产品网页

范围：
- **CI 在三个系统上通过**（提交 `9779500`，Windows、macOS、Ubuntu 24.04）。从改名到这里，CI 一共抓到 6 个本机抓不到的问题，见下表。
- **产品网页** `site/`：静态 HTML、CSS、JavaScript，没有构建步骤，字体、图片、视频都随页面提供。方向是"护照"：每个功能一页，真实截图像证件照一样贴在页上，印章上是一个实测的数字，底部的机读区放安装命令。英文为主，可切换中文；浅色和深色两套。`.github/workflows/pages.yml` 把它发布到 GitHub Pages。
- **`tools/site-check.ps1`**：7 项检查把网页和代码对上（每段文字都有中文、引用的文件都在、不从别的主机加载东西、命令名就是 `ut --list` 的输出、安装命令和 README 逐字相同、没有本机路径和邮箱、首次访问不超过 1500 KB）。已接进 `tools/round.ps1`。
- **`tools/scripts-check.ps1`**：解析仓库里全部 `.ps1`（14 个）、`.sh`（5 个），以及 4 个工作流里 35 个步骤的脚本，按每个步骤实际用的 shell（共 40 次检查）。也接进了 `tools/round.ps1`，一轮验收从 14 项变成 16 项。
- **`tools/promo.ps1 -Stills DIR`**：从真实程序录下网页用的 6 张截图，每张裁到有内容的那几行。
- `DESIGN.md` 和 `.impeccable/design.json`：从做好的网页整理出的设计规则。

### 发现并修掉的问题（每次发现后干净轮数归零）

| # | 视角 | 问题 | 定性 | 处理 |
|---|---|---|---|---|
| 1 | CI（Rust 1.100.0） | 新版 clippy 多了一条 lint `assert_is_empty`，本机 1.98.0 没有。 | 结构缺陷：工具链版本没有固定，CI 和本机用的不是同一个 | `rust-toolchain.toml` 固定 1.98.0。 |
| 2 | CI（macOS） | `macos-14` 这个运行器已经不提供。 | 边界遗漏 | 改用 `macos-latest`。 |
| 3 | CI（Linux） | `typed_text_reaches_the_program` 偶尔失败：它认定某一行就是输入的那一行。 | 测试的假设不成立 | 接受任何一行以 `42` 结尾且不是输入行本身。 |
| 4 | CI（Windows） | WiX 5 要求自动生成 GUID 的组件里只能有一个文件。 | 边界遗漏 | `unifyterm.wxs` 改成一个文件一个组件。 |
| 5 | CI（Windows） | `vendor/conpty/LICENSE` 在被 git 忽略的目录里，CI 上没有这个文件。 | 结构缺陷：通知文件依赖一个不在仓库里的文件 | 许可证文本放到 `third-party/conpty/LICENSE`，`notices.ps1` 从那里读。 |
| 6 | CI（Linux） | linuxdeploy 生成的文件叫 `UnifyTerm-…AppImage`，脚本找的是小写名字。 | 边界遗漏 | `appimage.sh` 取它实际生成的那个文件。 |
| 7 | 网页：看截图 | B612 Mono 把句点和冒号放在字格左边，网址读成 `https: //raw. github…`。 | 共性：所有命令和网址 | 等宽字体换成 Overpass Mono，删掉 B612 Mono 的两个文件。 |
| 8 | 网页：算对比度 | 浅色纸上，青色印章 3.33:1，金色 3.74:1，低于 4.5:1。 | 边界遗漏 | 两个颜色加深，最低变成 5.84:1 和 5.72:1。 |
| 9 | 网页：看截图 | 印章盖在深色截图上看不清；截图下半截是空的。 | 结构缺陷：印章的位置依赖截图的颜色 | 印章放在截图下方的纸上，只压住一个角；截图裁到有内容的行。 |
| 10 | 网页：看截图 | 双栏时，99 个字符的安装命令在单词中间断行（`mai` / `n`）。 | 边界遗漏 | 安装命令改成单栏。 |
| 11 | 网页：量尺寸 | "全部 99 个命令"这个展开按钮只有 21 px 高。 | 边界遗漏 | 加上下内边距。之后 5 种宽度下小于 24 px 的点击目标都是 0。 |
| 12 | 网页：没有 JavaScript | 语言、主题、标签、复制这几个控件点了没反应。 | 边界遗漏 | 没有脚本时不显示它们；页面内容和 4 条安装命令照样完整。 |
| 13 | 发布（第一次打 `v0.1.0`，工作流第一步就停了） | `release.yml` 内嵌的 PowerShell 里写了 `"… on $sha: fix …"`，`$sha:` 被当成带作用域的变量，脚本解析不了。没有生成 Release。 | 共性：同一种写法第三次出现（前两次在 `install.ps1`、`package-manifests.ps1`）。检查有结构缺陷：只解析 `.ps1` 文件，不看工作流里的脚本 | 改成 `${sha}`；全仓再搜这种写法，剩 0 处。新增 `tools/scripts-check.ps1`：修之前报 1 处，修之后 0 处。 |
| 14 | 发布（通读 `release.yml`） | 等 CI 的那一步只看这个提交上最新的一次运行，而最新那次正是标签自己触发的：已经通过的提交也要再等一整遍 CI（上次 56 分钟，期限 60 分钟）；那次运行若被取消，发布就失败。 | 结构缺陷：把"最新一次"当成"这个提交的结论" | 有任何一次通过就算通过；全部结束且没有一次通过才停；期限 90 分钟。CI 不再对标签运行。用真实仓库试了 3 个提交：通过 + 取消 → 通过；只有失败 → 停；没有运行 → 等。 |
| 15 | 发布（通读 `release.yml`） | `gh release create $tag dist/*.zip …` 写在 PowerShell 里，PowerShell 不替程序展开 `*`。`gh` 自己会不会展开没有确认。 | 边界遗漏（未证实的风险，按会出错处理） | 逐个列出文件再传给 `gh`，并核对正好 4 个。 |

检查工具自身的问题：
- 无头 Edge 是图形界面程序，PowerShell 用 `& $edge` 启动它不会等它结束，`--dump-dom` 的输出因此读成空的，两次。改用 `Start-Process -Wait -RedirectStandardOutput`。
- 无头 Edge 的窗口最窄约 500 px：`--window-size=390` 截出来的图是 390 px 宽，排版却是按 500 px 做的，看上去像横向溢出。改成把页面放进 390 px 宽的 iframe 里量。
- 无头 Edge 的虚拟时间不产生画面帧，`IntersectionObserver` 不触发，"印章落下"在那里测不了。改在真实的 Chrome 里测。那个标签页每秒只渲染 2 帧（窗口被挡住），等到有帧之后 9 个印章全部落下。

### 连续三轮干净

第 13 条是在三轮干净之后、打标签时发现的，所以之前那三轮作废，从头再计。下面是修完第 13 到 15 条之后的三轮；CI 在修复提交 `b93e5ff` 上三个系统都通过。

| 轮 | 视角 | 构建 | 测试 | 实机 | 其他数字 |
|---|---|---|---|---|---|
| 1 | 机制通路存活 | debug | 207/207 | 110/110，8 个小节都跑完 | `round.ps1` 16/16；Linux 60/60；p95 3.24 ms；脚本检查 3/3，它在修之前能抓到第 13 条（1 处），修之后 0 处；等 CI 的判断在 3 个真实提交上结果都对；网页 7/7；历史文件 86745 → 86745 字节 |
| 2 | 静态一致性 | release | 207/207 | 110/110，8 个小节都跑完 | `round.ps1` 16/16；Linux 60/60；p95 3.18 ms；`$名字:` 这种写法在全部 `.ps1` 和工作流里 0 处；4 个工作流 35 个步骤的脚本 40 次检查全部能解析；`release.yml` 依赖的名字和生成它们的脚本一致（3 个压缩包名、dmg 名、AppImage 名、`package-manifests.ps1` 的 3 个参数） |
| 3 | 可复现性 | — | — | — | 第 1、2 轮的实机输出各 110 行，去掉计时和临时目录名里的进程号后 0 处不同；`scripts-check` 和 `site-check` 各连跑 3 次，输出相同；`ut --version` 两次相同 |

第 3 轮没有再跑一遍全量测试：它比较的是前两轮的输出。

作废的那三轮的数字（修第 13 条之前）：release 15/15、debug 15/15，测试 207/207，实机 110/110；网页的数字见下。

网页的测量（在最终的页面上做，第 13 到 15 条没有改网页）：
- 行为：`site-check` 7/7，故意改错 6 处、6 处都被抓到；中文切换改了 53 处，切回后逐字相同；主题来回切换，纸的颜色 `rgb(20, 26, 48)` ↔ `rgb(242, 245, 250)`；4 个标签各自显示自己的命令，左箭头从第一个转到最后一个；剪贴板被拒绝时命令被选中；真实 Chrome 里 9 个印章都落下。
- 尺寸：320、390、768、1440 px（5 种语言和主题的组合），横向溢出 0，小于 24 px 的点击目标 0，4 个字体都加载。
- 对比度：浅色文字最低 5.48:1，印章墨色最低 5.72:1；深色最低 6.58:1。
- 去掉两段脚本后页面完整，脚本才能用的控件不显示。
- 整页截图两次哈希相同（`8E9DD4DC4A90`）；`DESIGN.md` 的 22 个颜色都在 `style.css` 里。
### 没有在本机验证的

- MSI 的构建：下载 WiX 被权限系统拒绝，只在 CI 和发布工作流里构建。
- `release.yml` 第一步之后的各步：只有打标签才会运行，本机只能检查语法和它依赖的名字。在 macOS 的 arm64 机器上交叉构建 x86_64、在 Ubuntu 22.04 上构建，这两件事 CI 没有做过。
- 网页在 Safari 和 Firefox 里的样子：只在 Edge（无头）和 Chrome 里看过。`mask`、`color-mix`、`:has()` 这三个 CSS 特性需要 2023 年以后的浏览器。
- 网页的"复制"成功那条路：测试不写用户的剪贴板，只测了剪贴板被拒绝时选中命令。
- 上一条记录里"没有在本机验证的"各项仍然成立。
## 2026-11-18 · 改名为 UnifyTerm；scoop、brew、curl 安装；宣传视频

本条之前的记录都用旧名 uni-term，保持原样不改写。

范围：
- **改名**：uni-term → UnifyTerm。给人看的地方（窗口标题、提示文字、安装器、开始菜单、`.desktop`、`.app`、winget、文档正文）写 `UnifyTerm`；给机器用的地方（crate、可执行文件、目录、`TERM_PROGRAM`、MCP 服务名、skill 名、socket 名）写全小写 `unifyterm`。内部库改为 `unifyterm-history`、`unifyterm-assist`；环境变量 `UNI_TERM_*` 改为 `UNIFYTERM_*`。命令名 `ut` 保留。
- **新 logo**：三股线（三个平台）汇成提示符 `>`，后面是光标。`docs/brand/`、`packaging/icons/`。
- **安装方式**（与 keepane 相同的结构，并加上一键脚本）：
  - `brew install newdee/tap/unifyterm`：`packaging/homebrew/` 里是要放进 `newdee/homebrew-tap` 的 formula、跟版本脚本和测试工作流。
  - `scoop install <raw 地址>`：`packaging/scoop/unifyterm.json`，发布时由工作流填入版本和哈希并提交回 `main`。
  - `curl -fsSL …/install.sh | sh`（Linux、macOS）和 `irm …/install.ps1 | iex`（Windows）：下载最新版本，校验 SHA-256，只为当前用户安装，不需要管理员或 root。
  - `.github/workflows/release.yml`：打 `v*` 标签后，等 CI 通过，构建全部安装包，创建 Release，更新清单。
- **宣传视频**：`tools/promo.ps1` 用真实运行画面逐帧录制，生成 `docs/media/unifyterm-promo.mp4`（46 秒）和 `.gif`（960×540，2.4 MB），放在 README 顶部。

旧数据目录 `%LOCALAPPDATA%\uni-term` 里只有会自动重建的命令链接、shell 钩子和日志，没有历史库和配置，所以没有写迁移代码。改名后它不再被使用，删不删由用户决定。

### 发现并修掉的问题（每次发现后干净轮数归零）

| # | 视角 | 问题 | 定性 | 处理 |
|---|---|---|---|---|
| 1 | 机制通路（实测抓到） | `install.ps1` 根本无法解析：字符串 `"…running from $dir: close it first"` 里，PowerShell 把 `$dir:` 当成带作用域前缀的变量。 | 共性：同一种写法 | 改成 `${dir}`。用解析器检查全部 10 个 PowerShell 脚本，又发现 `package-manifests.ps1` 里的 `$Version:`，一并修掉。之后 10/10 解析通过。 |
| 2 | 静态一致性 | 机械替换后，有的地方大小写不对：`actions!(UnifyTerm, …)` 是标识符，应为小写；skill 的 `name:` 和 `TERM_PROGRAM=` 被改成 UnifyTerm，应为小写；版本测试和两条实机检查还在找小写的旧格式。 | 替换规则的边界 | 逐处改正；第 2 轮用脚本核对 11 个机器名都是 `unifyterm`、8 个显示名都是 `UnifyTerm`。 |
| 3 | 隐私（看视频时发现） | 第一版宣传视频里，提示符和输入的路径都是 `C:/Users/<用户名>/Documents/…`，会把用户的 Windows 用户名和目录结构公开。 | 隐私缺陷 | 演示目录用 `subst` 挂成一个单独的盘符，提示符只显示 `U:/`；演示用的 git 仓库作者为 `demo <demo@example.com>`。 |
| 4 | 内容（看视频时发现） | 第一版视频：字太小；历史一幕被长输出占满，候选列表没拍到；AI 解释是假模型的占位文字 "part N"。 | 演示质量 | 字号 20；预先跑输出短的命令；给假模型加 `-Meanings` 参数（不传时行为不变，实机检查不受影响），返回真实的解释。逐帧检查过：候选列表、Ctrl-N 高亮、Enter 填入、运行，都拍到了。 |

检查工具自身的问题：
- PowerShell 的 `-ne` 和 `Compare-Object` 默认不区分大小写：改大小写的那一步一度"改了 0 行"，金样对照的差异也一度显示为 0。改用 `-cne` 和 `-CaseSensitive`。
- 用 `Start-Process` 启动一轮验收收不到完成通知；没有中途杀掉它（上一次中途杀掉留下了坏的编译产物），而是另起一个等待结果文件的后台任务。
- 静态检查脚本里在 PowerShell 字符串中写了 bash 的 `\"`。

### 连续三轮干净

| 轮 | 视角 | 构建 | 测试 | 实机 | 其他数字 |
|---|---|---|---|---|---|
| 1 | 机制通路存活 + 边界输入 | debug | 207/207 | 110/110，8 个小节都跑完 | `install.ps1`：安装、覆盖更新、哈希不符时拒绝，PATH 和开始菜单未被改动；`install.sh`（dash）：安装、菜单项、图标、更新、哈希不符时拒绝；Windows 写的历史库在 Linux 上读出 4 条，相同；历史文件 87347 → 87347 字节 |
| 2 | 静态一致性 | release | 207/207 | 110/110，8 个小节都跑完 | 旧名字残留 0 处（109 个文件），文件名 0 处；机器名 11 个都是小写 unifyterm（scoop 的 `unifyterm.exe` 按 scoop 的要求带扩展名）；显示名 8/8 为 UnifyTerm；PowerShell 脚本 10/10 解析通过；sh 和 bash 脚本 4/4 通过 `-n`；scoop 清单是合法 JSON；工作流引用的 6 个文件都在 |
| 3 | 可复现性 | release | — | — | 通知文件生成两次哈希相同；`install.ps1` 装两次，7 个文件逐字节相同；版本号两次相同；第 1、2 轮实机输出 108 行 0 处不同 |

### 没有在本机验证的

- formula 的 Ruby 语法：WSL 里没有 ruby，由 tap 仓库 CI 的 `brew audit` 检查。
- `install.ps1` 修改 PATH 和建开始菜单快捷方式这两步：会改用户的配置，测试时关掉了，发布后按 `docs/release.md` 第 2 步验证。
- `install.sh` 用的是代替 `unifyterm` 的小脚本（本机链接不了 Linux 版 GUI），`ut` 是真的。
- scoop 安装：会改用户的 scoop 环境，发布后验证。

## 2026-11-18 · M7 视频、安装包、跨平台、发布准备

范围：
- `crates/ut/src/video.rs`：`ut view` 播放视频。ffprobe 取尺寸和帧率，ffmpeg 把帧解成 rgb24 送进管道，帧率最高 30。kitty 终端里每帧用同一个图片编号原地替换；其他终端用半块、盲文或 ASCII 文字重画。按时间戳定速，跟不上就丢帧。`q` 或 Ctrl-C 停止，恢复终端模式和光标，结束 ffmpeg。输出不是终端时只给第一帧。没有声音。
- 版本号带 git 提交：`uni-term 0.1.0 (c5a6513)`、`ut 0.1.0 (c5a6513, 99 commands)`。两个构建脚本共用 `build/commit.rs`。
- `LICENSE`（MIT）；`THIRD-PARTY-NOTICES`：917 个包、ConPTY 的许可证原文，由 `tools/notices.ps1`（cargo-about）生成；`about.toml` 列出允许的许可证。
- 打包：`packaging/windows/uni-term.wxs`（WiX v4/v5）和 winget 清单；`.deb` 配置（`crates/uni-term/Cargo.toml`）、`.desktop`、AppImage 脚本；macOS 的 `.app`/`.dmg` 脚本、`Info.plist`、Homebrew cask；图标（SVG、PNG、ICO）。
- `.github/workflows/ci.yml`：Windows、macOS、Linux 三个平台的格式、lint、测试、release 构建、许可证检查、窗口冒烟、打包、上传产物。
- `README.md`（安装、快捷键、配置项、环境变量、命令、数据位置、已知限制）；`docs/release.md`（发布清单）；`tools/release.ps1`（填 winget 和 cask 的地址、哈希、ProductCode）。
- `third-party/xattr-0.2.3`：改了一行的副本（见问题 2）。
- 检查：`verify.ps1` 的 graphics 一节加 3 项视频检查；`round.ps1` 加 3 项：Windows 写的历史库在 Linux 上读、README 与代码一致（`tools/docs-check.ps1`）、通知文件是最新的。

### 第 0 步：核实的事实

| 项目 | 结论 |
|---|---|
| ffmpeg 解码到管道 | 可以。ffmpeg 8.1.1 输出 20 帧 64×64 rgb24，颜色误差在 yuv 转换的 ±2 以内。 |
| WSLg 里运行 Linux 版 GUI | **不行。** WSL 里没有 `libxkbcommon-x11`，开发头文件也没有，sudo 要密码。Linux 版 GUI 改由 CI 构建和冒烟（apt 装开发包、xvfb、lavapipe）。 |
| 打包工具 | cargo-wix 能装，但 WiX 3 二进制的下载被权限系统拒绝，而且 WiX 3 已停止维护。改用 WiX v4/v5 的 `.wxs`，由 CI 用 `dotnet tool install wix` 构建。cargo-deb 在 CI 上安装。 |
| 许可证 | 917 个包全部在允许列表内。MPL-2.0 的 2 个（dwrote、option-ext）未修改，通知文件里写明源码地址。"Apache-2.0 OR GPL-2.0-only"、"…OR LGPL-2.1-or-later" 选择 Apache 或 MIT。没有与 MIT 发布冲突的许可证。 |

### 与 plan.md 不同的地方

| plan 写的 | 实际做法 | 原因 |
|---|---|---|
| 视频夹具 2 秒，0.5 秒和 1.5 秒截图 | 4 秒（红 2 秒、蓝 2 秒），1 秒和 3 秒截图 | 给 ut 和 ffprobe 的启动留出余量，截图点离颜色切换点各 1 秒。 |
| MSI 用 cargo-wix | WiX v4/v5 的 `.wxs`，`wix build` | WiX 3 停止维护；本机下载 WiX 被拒绝。 |
| 在干净的 Windows 里安装、卸载 MSI；在 WSL 干净发行版里 `dpkg -i` | 未做，列入 `docs/release.md` 第 2 步 | 本机没有 WiX，生成不了 MSI；`dpkg -i` 要 sudo。CI 生成包以后由用户验证。 |
| 把 Windows 的 `config.toml` 拷到 Linux 验证 | 只验证了历史库；配置的解析由 CI 在 Linux 上跑 uni-term 的测试覆盖 | uni-term 在本机 WSL 里链接不了（缺 libxkbcommon-x11）。 |
| 本地先用 `act` 跑 CI | `act --list` 只解析了工作流文件；Windows 冒烟一步在本机照原样跑通 | act 要 Docker 和 Linux 镜像，跑不了 Windows 和 macOS 的任务。 |

### 发现并修掉的问题（每次发现后干净轮数归零）

| # | 视角 | 问题 | 定性 | 处理 |
|---|---|---|---|---|
| 1 | 第 0 步 | GPUI 0.2.2 → `gpui_http_client` → `zed-async-tar` → xattr 0.2.3，它在 Linux 上用 `libc::ENOATTR`，而 libc 0.2.190 起 Linux 上没有这个常量：uni-term 在 Linux 上编译不过。libc 不能退版本（mio 1.2.5 要求 ≥ 0.2.190）。 | 依赖的结构问题（plan 风险一节预计到的"GPUI 不再更新"） | `[patch.crates-io]` 指向仓库内的 xattr 0.2.3 副本，只改一行：Linux 和 Android 上用 `ENODATA`（同一个值）。说明在 `third-party/xattr-0.2.3/UNI-TERM-CHANGES.md`，写明解除条件。之后 Linux 上编译通过，只剩链接缺系统库。 |
| 2 | 代码正确性 | ConPTY 在程序结束后还会异步画出它最后的输出，而 waiter 线程一看到程序结束就报告"结束"，标签页随即关闭，最后的输出可能丢失。第一次冒烟测试的原始日志在 `smoke-ok` 的 `sm` 处截断（53 字节）。 | 结构缺陷：两个线程没有先后保证，所有消费方都假设"结束报告在全部输出之后" | waiter 等程序结束后，再等 reader 读到 EOF 或输出静默 100 ms（最多 1 秒），然后才报告结束。修复后冒烟 140 次 0 次丢失。**证据有限**：撤掉修复后热启动 80 次也没有复现，只有那一次冷启动的截断；保留修复是因为它从结构上保证顺序，代价是关标签页最多晚 100 ms。 |
| 3 | 边界输入（review） | 视频的文字模式里，预留的行数用公式算，每帧实际的行数由解码高度决定，取整不同：braille 模式下实际可能比预留多 1 行（例：10 列、100×29 的视频，预留 1 行，实际 2 行），在屏幕底部会滚动，帧位置漂移。 | 边界遗漏 | 文字模式的行数直接从帧高度算（half/ascii：高度/2；braille：⌈高度/4⌉）。新测试对 4 种尺寸 × 3 种模式核对"预留行数 = 实际行数"；换回旧公式时它在 braille、10 列、100×29 上失败。 |
| 4 | 文档一致性（review） | README 有 7 处不准：命令数（写成 6 + 90）；"帮助在指定模型前关闭"（本地纠正一直开着）；winget 和 brew 写得像已经能装；`.deb` 文件名缺 `-1`；"Cmd 代替 Ctrl-Shift"（Ctrl-Shift-A 对应 Cmd-Shift-A）；点击卡片失败会复制位置没写；"发送给模型的内容"漏了"最近 5 条命令"。另外第一稿还有 Rust 版本（1.92，应为 1.93）、macOS 命令数、PowerShell 版本、keepane 链接 4 处，在核实时改掉。 | 文档与事实不符 | 逐句对照代码改正；发送内容改为照抄界面里征求同意时显示的原文。`tools/docs-check.ps1` 自动核对配置项、快捷键、环境变量三张表（删掉一行或编造一个键，它都会报错）。 |
| 5 | 检查自身 | `ut jq` 和 `ut view` 读 `NO_COLOR`，`ut jq` 读 `JQ_COLORS`，README 没提。 | 文档缺口 | README 加一句；检查脚本单独核对这两个通用变量。 |
| 6 | **可复现性（第 3 轮抓到）** | 两轮实机输出比较时，"侧栏显示 3 秒内查询了几次"一行不同（6 和 7）。这个数取决于墙钟时间，本来就会浮动，两次都在允许范围（4 到 8）内。问题在比较脚本：它排除了内存数字，却没有排除这一类计时数字。 | 检查脚本的结构缺陷（与 CLAUDE.md 里记的"把墙钟时间放进对照表"同类） | 比较时，计时类的行（内存、轮询次数）只比较判定结果，不比较数字。verify.ps1 里受墙钟影响的数字共 3 处，全部处理。 |

检查工具自身的问题：
- 用 `$r = & uni-term.exe --replay` 读 release 版的输出，拿到 0 行：release 版是窗口程序，PowerShell 不等它结束。这让我一度以为"10 次冒烟 10 次丢输出"，实际是判断方法的错。改成 `verify.ps1` 一直在用的 `Start-Process -Wait -RedirectStandardOutput`，CI 的冒烟一步也改了。
- 停掉一轮时 cargo 正在编译，留下损坏的增量产物，下一轮链接失败（LNK2001）。`cargo clean -p ut` 后恢复。
- `wsl.exe` 传参会弄乱引号，WSL 的提示行是带 NUL 的 UTF-16：脚本一律写成文件再运行，比较时只取符合格式的行。
- `Get-ChildItem README.md -Recurse` 会找到子目录里的同名文件，误报了 xattr 自己的文档地址。

### 连续三轮干净

| 轮 | 视角 | 构建 | 测试 | 实机 | 其他数字 |
|---|---|---|---|---|---|
| 1 | 机制通路存活 + 边界输入 | debug | 207/207 | 110/110，8 个小节都跑完 | 视频第 1 秒 #dc2727（夹具红 #dc2828）、第 3 秒 #264eda（夹具蓝 #2850dc）；按 q 后 0.9 秒内回到提示符，ffmpeg 残留 0；没有 ffmpeg 时退出码 1；Windows 写的历史库在 Linux 上读出 4 条，逐字节相同；README 4 张表一致；p95 3.31 ms；Linux 60/60；历史文件 87347 → 87347 字节 |
| 2 | 静态一致性 | release | 207/207 | 110/110，8 个小节都跑完 | 打包文件、脚本、CI 引用的 16 个文件全部存在；5 处版本号一致（0.1.0）；`OWNER` 占位 4 处，发布清单写明要替换；xattr 补丁的说明和许可证都在；ConPTY 许可证在通知文件里；非测试代码 `unwrap` 0、`allow` 0；`creation_flags` 1 处 |
| 3 | 可复现性 | release | — | — | 通知文件生成两次哈希相同（git 无改动）；视频第一帧经管道 3 次逐字节相同；窗口内视频 3 次在 1 秒和 3 秒的颜色相同；两个版本号两次相同；第 1、2 轮实机输出 108 行 0 处不同（计时类数字只比较判定） |

### 需要用户的

- 推送到远程仓库并启用 CI；macOS 的构建、测试和窗口冒烟只能在 CI 上跑。
- 把文件里的 `OWNER` 换成真实的账号（4 处）。
- MSI 在本机生成需要 WiX：允许下载 WiX，或只在 CI 上生成。
- 在干净环境里安装和卸载 MSI、`.deb`（`docs/release.md` 第 2 步）。
- 签名（Windows 代码签名证书、Apple Developer ID）和公证。
- 提交 winget 清单和 Homebrew cask；GitHub Release。
- 本机验证 Linux 版 GUI 需要 `sudo apt install libxkbcommon-x11-0 libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libwayland-dev libvulkan-dev libfontconfig-dev`。
- `LICENSE` 的版权方写的是 "the uni-term authors"，请确认是否改成你的名字或组织。

### 已知限制

- 视频没有声音；需要安装 ffmpeg。
- stdin 不是终端时（被重定向），播放中按 Ctrl-C 会直接结束 ut，光标可能停在隐藏状态。
- `tools/release.ps1` 没有实测过：它需要 CI 生成的 MSI 和 dmg。
- MSI、`.deb`、AppImage、`.dmg` 的构建步骤只写在 CI 里，本机没有生成过。

## 2026-11-18 · M6 agent 侧栏、`ut mcp`、skill

范围：
- `crates/uni-term`：
  - `agents.rs`：读取 `keepane list-agents -J` 的输出；未知字段忽略，缺少的字段取空值。运行命令时无窗口，最多 1 秒，超时就停掉。同一个问题 1 分钟最多写 1 次日志。
  - 侧栏（Ctrl-Shift-A，macOS 上是 Cmd-Shift-A）：每个 agent 一张卡片，显示 agent 和模型、所在窗格、费用、上下文占比、CPU、内存、轮数。点击卡片运行 `agents.focus`（默认 `keepane focus-pane %N`）；失败时把窗格位置复制到剪贴板。
  - `[agents]` 配置 4 项：`sidebar`、`poll_ms`、`command`、`focus`。
  - `shell::no_window`：无窗口启动子进程，原来分散在 2 处，现在 3 个调用方共用。
- `crates/ut`：
  - `ut mcp`：stdin/stdout 上的 JSON-RPC，4 个工具（`history_search`、`history_recent`、`list_commands`、`command_help`），协议版本 2025-06-18、2025-03-26、2024-11-05。历史库只读打开；库不存在时返回空列表，不创建文件。不提供运行命令的工具。
  - `ut skill`：打印 skill 文件；`ut skill --install 目录` 写入 `目录/uni-term/SKILL.md`。不自动写任何 agent 的配置目录。
- `skills/uni-term/SKILL.md`。
- `tools/verify.ps1` 新增 `agents` 一节，13 项。

### 与 plan.md 不同的地方

| plan 写的 | 实际做法 | 原因 |
|---|---|---|
| 夹具取自测试 keepane 服务里 2 个窗格的 `list-agents -J` 输出 | 夹具按真实输出的字段手写，数值虚构 | `list-agents` 只列出运行着 agent 的窗格。测试服务里没有 agent，输出是 `[]`。为了测试去启动真的 Claude Code，既花钱又需要登录。 |
| 侧栏固定运行 `keepane list-agents -J` | 新增配置 `agents.command` 和 `agents.focus` | 实机检查可以换成读夹具的命令；用户用自定义 socket 时也能改。 |
| 当前标签页运行 keepane 时切换窗格，否则复制 | 先运行 `focus-pane`，失败才复制 | `focus-pane` 会把所有已连接的客户端切到那个窗格并带到前台。keepane 在别的终端窗口里运行时，这样也能直接跳过去。 |
| 真实 agent 检查要先注册 MCP，会改用户的 Claude Code 配置 | 用 `claude -p --mcp-config 文件 --strict-mcp-config`，只对这一次会话有效 | 不改任何配置。数据目录是临时目录，里面只有 2 条虚构命令。 |

### 发现并修掉的问题（每次发现后干净轮数归零）

| # | 视角 | 问题 | 定性 | 处理 |
|---|---|---|---|---|
| 1 | 代码正确性（测试抓到） | 命令超时后进程被停掉了，但它启动的子进程还持有输出管道，读管道的线程一直等到子进程自己结束（实测 3 秒，超时设定为 1 秒）。 | 边界遗漏；同一个假设"进程结束，管道就关闭"在正常退出路径上也不成立 | 读管道的结果按同一个截止时间收取，两条路径都覆盖。新增测试：命令立即退出，但留下一个持有管道的后台进程，结果也是"超时"。 |
| 2 | 逻辑正确（review） | 点击卡片运行的 `focus` 命令没有超时，卡住就永远占着一个后台线程。`ask` 有超时，两个函数各写了一套启动子进程的代码。 | 结构缺陷：两份实现，健壮程度不同 | 收敛成一个带超时的 `run`，`ask` 和 `focus` 共用。新增测试：卡住的 focus 命令在超时后报错。 |
| 3 | 边界输入（review） | 侧栏关闭再打开，第一次查询返回之前仍显示上一次的列表。 | 边界遗漏 | 打开时清空，先显示 "Asking keepane…"。 |
| 4 | 机制通路（测试抓到） | `command_help` 对 `test` 返回空：按 POSIX，`test --help` 什么都不打印。 | 个性 | 改取 `[ --help`，两者是同一个命令。99 个命令全部拿到帮助（有测试）。 |
| 5 | 静态一致性（review） | `mcp.rs` 里一条走不到的分支写死了 20，没有用常量。 | 个性 | 改掉。全仓搜索同类写法，0 处。 |
| 6 | 共性（实现前） | "无窗口启动子进程"在 `helper.rs`、`shell.rs` 各写了一遍，侧栏会是第 3 处。 | 结构缺陷 | 收敛为 `shell::no_window`。非测试代码里的 `creation_flags` 只剩 1 处。 |
| 7 | 检查自身（review） | "没有 keepane 服务时只记录一次"的检查用默认 2 秒间隔，3 秒内只问了约 2 次，节流几乎没有被考验；说明里还写着"每秒两次"。 | 检查缺口 | 改为 500 ms 间隔：3 秒内问 6 次，日志只能有 1 行。 |
| 8 | **可复现性（第 3 轮抓到）** | 两轮实机检查里，测试 keepane 服务的窗格数一次是 4、一次是 1（应为 2）。原因有两个：(a) keepane 停止服务时会保存会话，下次恢复，我的测试在测试 socket 上留下了 3 个已保存会话；(b) 无尺寸的分离会话太小，`split-window` 失败。另外，keepane 为测试 socket 上的每个 PowerShell 窗格都复制了一份用户的 PSReadLine 历史，共 43 份，留在 `%LOCALAPPDATA%\keepane\sessions-uni-term-test`。 | 检查脚本的结构缺陷：没有确认测试环境准备成功，也没有清理到位 | 清理：逐个关闭会话、删除已保存记录，再删掉测试 socket 的目录。用户默认 socket 的会话没有动过，用户的历史文件仍是 87347 字节。检查改为：开始前清空；会话带尺寸；断言窗格正好 2 个；结束后检查目录没有残留。 |

检查工具自身的问题：
- `cmd /c echo` 会吃掉 JSON 里的引号，测试改为让 `findstr`/`cat` 直接读文件。
- 我在 PowerShell 双引号字符串里写了 `` `a ``，被转成了响铃字符，写进了 Rust 注释（clippy 抓到）。修后全仓 0 个。

### 连续三轮干净

| 轮 | 视角 | 构建 | 测试 | 实机 | 其他数字 |
|---|---|---|---|---|---|
| 1 | 机制通路存活 + 边界输入 | debug | 198/198 | 107/107，8 个小节都跑完 | 侧栏隐藏 6 秒运行 0 次，显示 3 秒（500 ms 间隔）运行 7 次；没有服务时 3 秒问 6 次只记 1 行日志；点击后键盘仍进终端；测试 keepane 2 个窗格、结束后无残留；p95 3.27 ms；Linux 53/53；历史文件 87347 → 87347 字节 |
| 2 | 静态一致性 | release | 198/198 | 107/107，8 个小节都跑完 | 30 个配置项全部被读取、全部有实机检查设置它；MCP 工具在文档、`tools/list`、应答三处一致（4 个）；skill 文件的命令数与 `ut --version` 一致，`ut skill` 输出与文件逐字节相同；`creation_flags` 只剩 1 处；非测试代码 `unwrap` 0、`allow` 0 |
| 3 | 可复现性 | release | — | — | 同样的 6 条 MCP 请求跑 3 次，输出哈希相同；侧栏截图 3 次，两两 0 像素差异；`ut skill` 两次相同；第 1、2 轮实机输出 104 行 0 处不同 |

### 额外的验证

- **真实 agent**：Claude Code（claude-haiku-5-5）通过 `ut mcp` 的 `history_search` 查询 "deploy"，回答 `echo deploy-staging-5520`，正确。
- **真实 keepane 数据**（只读）：默认 socket 上 `keepane list-agents -J` 返回 3 个 agent，侧栏显示 3 张卡片。记录里只有数字，没有内容。

### 已知限制

- 侧栏没有在"测试服务里有真 agent"的条件下自动检查过：卡片的检查用虚构夹具，真实数据只做了一次只读的手工核对。
- 命令的错误输出不是 UTF-8 时（例如 cmd 的中文提示），侧栏里的那行字会是乱码。keepane 自己输出 UTF-8，不受影响。
- `ut mcp` 不支持 JSON-RPC 批量请求（2025-03-26 版允许，2025-06-18 版已去掉）。
- 侧栏宽度固定为 300 像素。
- 把 `ut mcp` 注册进用户自己的 Claude Code（`claude mcp add uni-term -- ut mcp`）需要用户来做。

## 2026-11-18 · M5 图片与 markdown

范围：
- `crates/uni-term`：
  - `graphics.rs`：kitty graphics 协议。支持 `a=t/T/p/d/q`，`f=100/32/24`，`t=d`，分块传输 `m`，`c`/`r`/`C`/`U`/`q`。单张图片上限 64 MiB，合计 256 MiB，超出时先删最久没用到的。
  - 图片用 Unicode 占位字符 U+10EEEE 放进网格，所以随文字滚动、清屏时一起消失。行号和列号用 kitty 的 297 个变音符号表示。
  - `marks.rs` 识别 APC 序列。`frame.rs` 找出图片区域；占位格子不画文字。
  - `terminal.rs` 用 GPU 画图片，裁剪到可见的格子；纹理按显示尺寸缩放。
- `crates/ut`：
  - `ut view`：kitty → iTerm2 → sixel → 半块 `▀` → 盲文 → ASCII，自动选择；`--mode`、`--width`、`--verbose`。
  - `ut md`：markdown 渲染成带样式的文字；表格按显示宽度对齐，中文按 2 列计；链接用 OSC 8；不是终端时输出纯文本。
  - 金样 `tests/golden/pictures.txt`，Windows 和 Linux 共用。
- `tools/verify.ps1` 新增 `graphics` 一节，10 项；`tools/shot.ps1` 新增 `{MEM:name}`。

### 发现并修掉的问题（每次发现后干净轮数归零）

| # | 视角 | 问题 | 定性 | 处理 |
|---|---|---|---|---|
| 1 | 机制通路（实机检查抓到） | 30 张 2048×2048 的图片显示在 4 列宽的格子里，进程内存多了 533 MiB，超过上限（256 + 100）。原因：每张图都按原尺寸做了 GPU 纹理，图片库的上限管不到纹理。 | 结构缺陷：纹理尺寸跟原图走，不跟显示尺寸走 | 纹理按显示尺寸（格数 × 格子像素）缩放；尺寸变了就重做。修后多 262 MiB。 |
| 2 | 边界输入（review 抓到） | 图片的右边缘碰到屏幕最后一列时，第 2 行起向左偏一列。原因：每行写完后用"左移 N 列"回到行首；光标停在最后一列等待换行时，少移一列。 | 边界遗漏 | 改为"下移一行、移到第 S 列"（CHA），不依赖光标停在哪里。加了一条测试；把修复临时换回旧写法，这条测试会失败。 |
| 3 | 静态一致性 | 新测试里有一个多余的 `format!`，clippy 报错。这次修改后只跑了测试，没跑 clippy。 | 个性 | 改成字节字面量。 |
| 4 | 可复现性（第 3 轮抓到） | 两轮实机检查的输出有 4 行不同：四个色块的像素数顺序不同。原因：`verify.ps1` 用普通哈希表存色块，遍历顺序每次运行都可能变。 | 检查脚本的结构缺陷 | 改用有序表，输出里写明颜色名。全脚本没有别的地方遍历普通哈希表并输出（0 处）。 |
| 5 | 静态一致性（对照 plan.md） | plan.md 的 M5 验证清单里有 3 项没做到：图片按行移动的距离没验证（只验证了滚出屏幕）；Windows Terminal 自动选 sixel 没测；可复现性只截了 2 次图（要求 3 次）。 | 检查缺口 | 补上：屏幕满时输出 3 行，图片上移 76 px = 4 行 × 19 px；在 Windows Terminal 里 `--verbose` 报告选了 sixel；截图 3 次。 |
| 6 | 单元测试 | 夹具角上的像素在白色对角线上，测试对它的颜色假设错误；滚动测试滚的行数不够。 | 测试自身的错误 | 改为检查四个色块的颜色；滚够行数。 |

检查工具自身的问题：
- 窗口变小后，颜色计数越出截图边界，脚本报错（已限制在截图内）。
- 检查脚本里我自己的函数 `H` 被 PowerShell 的内置别名 `H`（`Get-History`）遮住。M3 时也出过同类错误（`History`）。

### 连续三轮干净

| 轮 | 视角 | 构建 | 测试 | 实机 | 其他数字 |
|---|---|---|---|---|---|
| 1 | 机制通路存活 + 边界输入 | debug | 186/186 | 95/95，7 个小节都跑完 | p95 3.17 ms；99/99 个命令启动；Linux 46/46；bash、zsh、fish 标记齐全；GNU grep 104/104；历史文件 87347 → 87347 字节；图片显示时多用内存 261 MiB（上限 356） |
| 2 | 静态一致性 | release | 186/186 | 95/95，7 个小节都跑完 | 文档表里的 13 个协议键全部被代码读取；6 种显示方式都有名字、都在 `--help` 里；变音符号表 297 行；非测试代码 `unwrap` 0、`allow` 0；`unsafe` 5 处全部带 SAFETY；`image` 库只有 1 个版本；Linux 上金样对照 3/3；plan 的 9 项实机检查都在 |
| 3 | 可复现性 | release | 金样对照连跑 3 次都通过 | — | kitty、sixel、iTerm2 输出各跑两次逐字节相同；同一张图截图 3 次，两两 0 像素差异（容差 2/255）；第 1、2 轮的实机输出 92 行 0 处不同 |

### 已知限制

- **image.nvim 没有验证。** 需要在用户的 nvim 里装插件，属于手工检查，留给用户。
- 只支持 kitty 协议的 Unicode 占位方式（`U=1`，以及我们自己的 `a=T` 放置）。不支持按像素坐标放置、z 轴层次和动画帧。
- uni-term 只解析 kitty 协议，不解析 sixel 和 iTerm2 协议。`ut view` 在 uni-term 里用 kitty。
- 纹理按显示尺寸缩放。同一张图在很多个不同尺寸的位置显示时，只保留最后一个尺寸的纹理。
- 第 5 项实机检查用了 30 张 16 MiB 的图（合计 480 MiB，超过 256 MiB 上限），不是 plan 里写的 100 张：30 张已经越过上限，再多只会让检查更慢。

## 2026-11-18 · M4 AI：建议、纠正、解释

范围：
- `crates/assist`（新库 `uni-assist`）：
  - `Provider` 接口。Anthropic Messages API 用原始 HTTP（`2026-10-21`，结构化输出，Fable 5.1/5.2、Opus 5/5.5、Sonnet 5.5 自动带 `fallbacks: "default"`）；另有 OpenAI 兼容接口，给本地模型等非 Claude 后端用。
  - 建议、纠正、解释三个功能，各自校验答案。
  - 本地纠正：OSA 编辑距离。
  - `clean()`：脱敏，并把用户主目录换成 `~`。
- `crates/history`：`redact()`，与入库过滤共用同一组规则。
- `crates/uni-term`：
  - `helper.rs`：后端、首次确认、在 PATH 里找程序名。
  - `[ai]` 配置 7 项。
  - 界面：灰色建议、纠正弹窗、解释弹窗（Ctrl-Shift-E）、首次确认弹窗。
- `tools/fake-model.ps1`：本机的假模型服务。`tools/verify.ps1` 新增 `ai` 一节，共 11 项。
- `demo/index.html`：快捷键和按键行为与实现一致；路径改用 `/`。

默认不启用任何后端。没有后端时，只有本地纠正。

### 发现并修掉的问题（每次发现后干净轮数归零）

| # | 视角 | 问题 | 定性 | 处理 |
|---|---|---|---|---|
| 1 | 正确性（测试抓到） | 脱敏时按规则依次替换：`Authorization: Bearer <token>` 先被规则 1 换掉了 "Bearer"，规则 3 就再也找不到 token，token 原样发了出去。 | 结构缺陷：结果取决于规则的顺序 | 所有规则都在原文上找出匹配区间，合并后一次替换。 |
| 2 | 界面 | 确认框的说明被截断（"...for a command that was"）：用户看不全会发送什么，就被要求同意。 | 边界遗漏（所有弹窗共用一个绘制函数） | 按显示宽度折行，中文按 2 列计；有 6 条测试。 |
| 3 | 代码正确性 | 纠正和建议的错误被悄悄吞掉：密钥错了也没有任何记录。 | 共性（两条路径） | 写进日志。实机检查：用一个错的密钥，日志里出现 "suggestion: ... 401 ... invalid x-api-key"。 |
| 4 | 代码正确性 | `ut <命令> --help` 在界面线程上同步运行，每次按 Ctrl-Shift-E 窗口都卡一下。 | 结构缺陷：放错了线程 | 改为在请求所在的后台线程里运行。 |
| 5 | 一致性 | `ai.suggest`、`ai.explain`、`theme.foreground` 没有任何实机检查会去设置它们。`theme.foreground` 是 M1 起的缺口：之前的统计只查"单词出现过"。 | 检查缺口 | 补上检查。统计改为查 "`名字 =`"。 |
| 6 | 一致性 | `explain = false` 时提示 "No model is set"，与事实不符。 | 边界遗漏 | 两种情况分开说明。 |
| 7 | **验收脚本** | 有一次第 2 轮的实机检查只有 74 项（应为 85 项），`ai` 一节整个缺失，可是 `round.ps1` 照样报告通过：它只数 PASS 和 FAIL 的行数，并且丢掉了 `verify.ps1` 的错误输出。缺失的原因已无从查起，之后也没有再出现。 | 结构缺陷：没有确认检查脚本完整跑完 | 保存完整输出，包括错误流；要求最后一行是 `0 failed` 且 6 个小节都在，否则不通过。 |
| 8 | demo | 选中候选后，demo 按 Enter 会运行命令；Tab 在没有选中时会取第一条；解释键是 Ctrl-E；路径用 `\`。 | 与实现不一致 | 改为与实现一致；在 browse 里验证：Enter 填入后输出 0 条，再按 Enter 输出 2 条，控制台报错 0 条。 |

检查工具自身的问题：第 3 轮对比两轮数字时，临时目录名里的进程号（`uni-term-verify-<PID>`）让 8 行看起来不同；去掉进程号后 83 行 0 处不同。

### 连续三轮干净

| 轮 | 视角 | 构建 | 测试 | 实机 | 其他数字 |
|---|---|---|---|---|---|
| 1 | 机制通路存活 + 边界输入 | debug | 165/165 | 85/85，6 个小节都跑完 | p95 3.21 ms；99/99 个命令启动；Linux 36/36；bash、zsh、fish 各 8/8 标记；GNU grep 104/104；历史文件 87347 → 87347 字节 |
| 2 | 静态一致性 | release | 165/165 | 85/85，6 个小节都跑完 | 26 个配置项全部被读取、全部有实机检查设置它；非测试代码 `unwrap` 0、`allow` 0；`unsafe` 5 处全部带 SAFETY；API 版本号和 fallback 标志各只定义一处；没有在代码里写死的模型名；密钥不进日志 |
| 3 | 可复现性 | release | 3 次结果相同 | — | 同样的输入跑两次，3 个请求（纠正、建议、解释）逐字节相同；解释框截图 0 像素差异；第 1、2 轮的实机数字 83 行 0 处不同 |

### 已知限制

- **没有用真实后端验证过。** 全部检查都用本机的假服务。真实的 Claude 后端需要用户提供密钥后，手工验证一次。
- 纠正只在输出看起来是"命令不存在"时出现。其他原因的失败（参数错了）不会弹出纠正。
- PowerShell 自带的灰色预测出现时，uni-term 的建议不显示（只在光标右边为空白时显示）。
- 每次请求都新开一个线程；连续快速停顿时可能同时有几个请求在途，过期的答案会被丢掉。
- `ut` 以外的程序不会去取 `--help`：运行它们可能有副作用。

## 2026-11-18 · M3 shell 钩子、命令历史、候选列表

范围：
- `crates/history`（新库 `uni-history`）：SQLite 历史库、密钥过滤、数据目录 `UNI_TERM_DATA_DIR`。
- `crates/uni-term`：`marks.rs`（标记扫描器）、`prompt.rs`（提示符状态）、`recorder.rs`（写库线程）、`candidates.rs`（候选列表）、`hooks/`（pwsh、bash、zsh、fish 的钩子和 cmd 的 `PROMPT`）、`[history]` 配置。
- `crates/ut`：`ut history`；`ut` 自己进入命令目录。
- `tools/verify.ps1` 的 `history` 一节、`tools/linux-test.sh` 的钩子检查、`crates/history/tests/speed.rs`。

检查手段：
- 测试 148 项，其中密钥测试集 100 行（50 行含密钥、50 行不含）。
- 性能测试：10 万条命令、1000 次查询，用固定的生成器。
- 实机检查 73 项，其中 history 一节 15 项。
- WSL 里 bash、zsh、fish 各自在 pty 中运行，检查 8 个标记。

### 发现并修掉的问题（每次发现后干净轮数归零）

| # | 视角 | 问题 | 定性 | 处理 |
|---|---|---|---|---|
| 1 | 性能 | 10 万条历史下 p95 83 ms（要求 5 ms），每次写入 3.8 ms。 | 结构：每次提交都刷盘；按时间排序要逐行回表；对每个候选单独查询 | `synchronous=NORMAL`；`(text, last_at)` 覆盖索引；FTS 按 rowid 倒序取；用一条 `json_each` 查询"在当前目录跑过"；`last_exit` 存进表。p95 降到 7.7 ms。 |
| 2 | 性能（实测定位） | 1 个字符的前缀（"c" 匹配 2.5 万行）要 4 ms；短前缀全排序是瓶颈。我先后猜了两个原因（IN 子查询全表扫描、WAL 未检查点），都被测量否定。 | 结构 | 生成列 `head1`、`head2` 加 `(head, last_at)` 索引；内存映射；每路召回 100 行。p95 降到 3.8 到 4.8 ms。 |
| 3 | 性能余量 | 有一轮 p95 测到 5.10 ms：余量太小，会时过时不过。 | 结构：3 个字符以上总要再跑一次 FTS | 前缀召回已满 100 行时不做 FTS。p95 降到 2.9 到 3.7 ms，中位数 1.1 到 1.3 ms。 |
| 4 | 正确性 | 密钥漏了两种：`npm_` 开头的 token，`--pass=x` 这个选项。 | 边界遗漏 | 补上这两种形状。 |
| 5 | 误伤 | `auth` 会匹配 `--author=`，正常的 `git commit --author=...` 不入库。 | 边界遗漏 | 改为 `auth\b` 加单列 `authorization`；测试证明 `MY_AUTH=x` 仍会被拒绝。 |
| 6 | 机制通路存活 | PowerShell 启动时执行的 `-Command ". 'bin.ps1'; . 'hook.ps1'"` 被记成一条命令。 | 边界遗漏：这条命令进入历史，晚于钩子记下历史编号 | 第一个提示符只记下编号，不报告命令。 |
| 7 | 界面 | 候选列表偏下约 2 行：用窗口坐标，去定位终端区域里的元素。 | 边界遗漏 | 改用终端区域的坐标。 |
| 8 | 机制通路存活 | cmd.exe 的钩子从未生效：只在 `PROMPT` 未设置时才设置它，而 Windows 的环境里默认就有 `PROMPT=$P$G`。 | 边界遗漏：前提不成立 | 包住用户已有的提示符，在前后加标记。 |
| 9 | 一致性 | `history.limit` 没有实机检查。 | 检查缺口 | 补上：`limit = 1` 时第 2、3 行区域差 7018 像素，第 1 行 0 像素。 |
| 10 | 一致性 | `ago()`、`now_ms()` 在两个程序里各有一份。 | 结构：重复实现 | 合并到 `uni-history`。 |
| 11 | **用户数据** | **测试写进了用户的 PowerShell 历史文件**，有两处来源：(a) M1 起就有的单元测试 `typed_text_reaches_the_program` 启动用户的 pwsh 并敲入 `Write-Output (6*7)`，每跑一次 `cargo test` 写一行；(b) 实机检查的规则是"指定了 `program` 就不是 PowerShell"，而 `powershell.exe` 那项检查恰好指定了它。 | 共性，结构缺陷：两个假设都不成立；验收脚本又只在实机检查前后比较字节数，覆盖不到 `cargo test` | (a) 测试改用 cmd.exe 和 sh；(b) 按程序名判断是否是 PowerShell；(c) 验收脚本改为在整轮前后比较字节数。用户历史里已有 4 行测试命令（2 行 `Write-Output (6*7)`、2 行 `echo powershell-one`），没有擅自删除。 |

检查工具自身的问题（同样记账）：
- 排序测试的期望顺序是我手算错的；改为写死按公式算出的分数：8.18、5.27、4.18、3.27、2.68、0.72。
- `History` 是 PowerShell 自带的别名，覆盖了检查脚本里同名的函数。
- 检查脚本里的变量 `$failed` 正是统计失败数用的变量，被我赋成了字符串；如果没有报错，失败数会被悄悄清掉。
- WSL 的钩子检查把含空格和括号的 `PATH` 拼进命令串，导致 3 种 shell 全部"缺标记"；改用固定的 `PATH`。
- 第 3 轮第一次做时，有两项检查本身没有验证到东西：一项比较时去掉了命令文本，一项用 1 个字符查到空结果。已修正，第 3 轮整轮重做。

### 连续三轮干净

| 轮 | 视角 | 构建 | 测试 | 实机 | 其他数字 |
|---|---|---|---|---|---|
| 1 | 机制通路存活 + 边界输入 | debug | 148/148 | 73/73 | p95 2.91 ms（p50 1.08）；99/99 个命令启动；Linux 36/36；bash、zsh、fish 各 8/8 标记；GNU grep 104/104；用户历史文件 87243 → 87243 字节 |
| 2 | 静态一致性 | release | 148/148 | 73/73 | 非测试代码 `unwrap` 0、`allow` 0；两个程序里 `pub` 0；`unsafe` 5 处全部带 SAFETY；19 个配置项全部被读取、全部有实机检查；4 个环境变量都在帮助里；6 个钩子文件都会写出；p95 3.22 ms |
| 3 | 可复现性 | release | 3 次结果相同 | — | 同样的命令在两个新数据目录里历史相同（3 条，次数、退出码、文本都相同）；钩子文件两次写出哈希相同；同一历史文件查询 3 次结果相同；性能测试两轮都是 7712 个候选；第 1、2 轮实机 23 行数字 0 处不同 |

### 已知限制

- keepane 里的候选列表没做：要改 keepane（wmux 仓库），需要用户确认。
- **PowerShell 的 Tab 补全仍给出 `\` 路径。** M2 记录里说要在 M3 处理，但 M3 计划里没有列这一项，这次也没有做。要做的话，需要改写 PSReadLine 的 Tab 键处理，有误改字符串内容的风险，留待以后。
- 候选列表显示时，↑、↓ 归列表，不再交给 shell 翻历史（这是需求本身的规定）。
- 不知道是哪种 shell 时（钩子没有说），能显示列表，但不能填入。
- 回滚区已满时，如果输入过程中屏幕滚动，读到的输入位置可能有偏差。
- bash 的 `PROMPT_COMMAND` 是数组时（bash 5.1 起支持），钩子只会接上数组的第一个元素。
- 两个 uni-term 同时启动时会同时写钩子文件，没有加锁。
- PowerShell 的退出码：cmdlet 失败时记为 1；`$LASTEXITCODE` 可能是更早那条原生命令留下的值。
- GUI 只在 Windows 上实测；bash、zsh、fish 只在 WSL 里验证了标记输出，没有验证 GUI。

## 2026-11-17 · M2 命令集

范围：`crates/ut`（multi-call 程序：uutils coreutils 0.12 的 90 个命令，Unix 上另加 15 个；findutils、sed、diffutils；基于 ripgrep 搜索库的 GNU 兼容 `grep`；基于 jaq 的 `jq`；`which`；`--links`；Windows 上的 `/` 路径和通配符展开），`crates/uni-term/src/shell.rs` 与 `start.ps1`（命令目录放在 PATH 最前；PowerShell 启动脚本；`[commands]` 配置），`tools/round.ps1`、`tools/linux-test.sh`、`tools/verify.ps1` 的 commands 部分。命令数：Windows 99 个，Linux 114 个。

代码规范：按 BeautifulRust 执行（`cargo fmt`、`clippy -D warnings`、workspace lints）。初次接入时有 290 条 clippy 发现，已全部处理为 0。

检查手段：
- 单元测试和集成测试 111 项。其中 84 条命令记录，Windows 和 Linux 对比的是同一份文件，逐字节比较。
- `ut` 的 grep 与 GNU grep 3.12 对比 104 项，stdout、stderr 和退出码都比。
- 99 个命令各启动一次。
- `tools/verify.ps1` 实机检查 59 项，含 M1 的 30 项。

### 发现并修掉的问题（每次发现后干净轮数归零）

| # | 视角 | 问题 | 定性 | 处理 |
|---|---|---|---|---|
| 1 | 机制通路存活 | `diff --help` 报"unrecognized option"，退出码 2（uutils diffutils 没有）。 | 边界遗漏 | `diff` 自己回答 `--help` 和 `--version`；`--` 之后的 `--help` 当作文件名。 |
| 2 | 与 GNU 对比 | grep 有 4 处与 GNU 不同：`^*` 里的星号；`-m N` 之后不再输出 `-A` 上下文；`--exclude` 不作用于命令行直接给出的文件；`-m 0`。 | 边界遗漏 | 逐项修正。对比项从 49 增加到 104，0 处不同。 |
| 3 | 静态一致性 | 16 个配置项里有 5 个没有实机检查（font.family、terminal.padding、theme.cursor/selection/ansi）。 | 检查缺口 | 补 6 项检查，用主题里不存在的颜色确认像素只能来自配置。光标 152 像素，正好 8×19 一格。 |
| 4 | 用户新要求 | 终端里出现 Windows 的 `\` 路径。 | 新功能 | `pwd find realpath dirname du df mktemp readlink which` 输出 `/`；PowerShell 默认提示符显示 `/`；`UT_NATIVE_PATHS` 恢复 `\`。 |
| 5 | 机制通路存活 | 交互式 PowerShell 里 `pwd`、`find` 仍输出 `\`，而 `cmd /c pwd` 正确。原因：用户 profile 里的 "Coreutils for Windows" 片段重写每一行输入，把 cat、ls 等改去执行它自己的程序。**`Get-Command cat` 显示的是 uni-term 的程序，实际执行的却是另一个。** | 结构缺陷：之前"PATH 最前即生效"的假设不成立 | 启动脚本在 uni-term 的标签页里，把 uni-term 在用的命令名从该片段的列表中移除，不改用户的 profile。实机检查现在用"敲入命令后的实际输出"判定，不再只看 `Get-Command`。 |
| 6 | 跟进 #5 | 去掉该片段后，PowerShell 不展开 `*.txt`，Unix 用户的写法失效（那个片段原本靠调用方式让对方程序展开）。 | 结构缺陷 | `ut` 在 Windows 上按 Unix shell 的规则展开通配符。带引号的参数由启动脚本加 U+E000 标记，命令收到后去掉标记，原样使用。 |
| 7 | 代码正确性 | `ut` 启动的子命令（`find -exec`、`xargs`、`env`）也会展开参数，`grep 'a*'` 会被换成文件名。 | 共性，结构缺陷：错误假设"参数总来自交互式 shell" | 只保留一个机制：每个 `ut` 命令为它启动的程序设置 `UT_NO_GLOB`。`slash::rerun` 里逐个加标记的做法随之删除。测试已证明：去掉修复后测试失败。 |
| 8 | 边界 | `./*.txt` 丢了 `./`；`.*` 匹配到 `.` 和 `..`。 | 边界遗漏 | 与 bash 5.2 一致。4 个手工对比全部相同。 |
| 9 | 静态一致性 | `UNI_TERM_COMMANDS` 导出了，但没人读；启动脚本又用占位符写了一遍同一个目录。 | 同一个值有两个来源 | 启动脚本改读这个环境变量。变量缺失时标记逻辑不做任何事，在两个 PowerShell 版本里都验证过。 |
| 10 | 一致性 | `ut --links` 的提示、grep 遍历出错的提示仍是 `\`。 | 共性（`ut` 自己生成的路径，共 3 处） | 统一用 `slash::shown` / `found_name`。在 Windows 上用拒绝读取的临时目录实测：`grep: tree/sub/locked: Permission denied`。 |
| 11 | 机制通路存活 | 带 `-e`/`-l`/`-c` 的 grep 着色、`which` 输出 `cat.EXE`、`[` 收到的 argv0 带 `.exe`、PATH 未设置时出现空项（等于当前目录）。 | 边界遗漏 | 验收前已修，此处补记。 |

检查工具自身的问题（同样记账）：
- **实机检查把敲入的命令写进了用户自己的 PowerShell 历史文件。** 现在每个测试 shell 先敲一行 `Set-PSReadLineOption -HistorySaveStyle SaveNothing`，这一行本身带 "password"，所以 PSReadLine 也不保存它。每轮检查前后比较历史文件的字节数：87034 → 87034。
- 剪贴板是全机共享的。另一个程序在检查期间写剪贴板，导致 1 次误报。现在最多重试 3 次，结果里写明第几次。
- `round.ps1` 的 `$flag = if (...) { @('--release') }` 只有一个元素时变成字符串，`@flag` 按字符拆开传参，于是 release 构建失败。**我起初把这次失败判断为"8 个并行任务下 rustc 崩溃"，写进了脚本注释并告诉了用户；这个判断是错的。** 已更正。rustc 真实崩溃只在 32 个并行任务时出现过两次。
- 启动 uni-term 的 `-Type` 里少写 `{}`，而且命令解析到了系统装的 Microsoft coreutils `echo`（它也自己展开通配符），导致测试没有测到要测的东西。现在一律用 `ut` 的完整路径。

### 连续三轮干净

| 轮 | 视角 | 构建 | 测试 | clippy | 实机 | 其他数字 |
|---|---|---|---|---|---|---|
| 1 | 机制通路存活 + 边界输入 | debug | 111/111 | 0 | 59/59 | 99/99 个命令启动；Linux 33/33，84 条记录 0 处不同；GNU grep 104/104；历史文件 87034 → 87034 字节 |
| 2 | 静态一致性 | release | 111/111 | 0 | 59/59 | 非测试代码 `unwrap` 0、`allow` 0、`pub` 0；`unsafe` 5 处全部带 SAFETY；16 个配置项全部被读取、全部有实机检查；105 个 uutils 依赖与命令表一一对应；启动脚本 3 个占位符全部替换 |
| 3 | 可复现性 | 两者 | 3 次 4/4 | — | — | 命令记录连跑 3 次与 golden 逐字节相同；debug 与 release 的 `--list`、`--help` 相同；99 个命令的帮助连跑 2 次哈希相同（00B0A01F3A28FF76）；`--links` 第二次 made 0 / kept 99；通配符连跑 3 次结果相同（17 个名字）；启动脚本重写后哈希不变；GNU grep 对比连跑 2 次输出相同；第 1、2 轮实机数字 0 处不同 |

### 已知限制

- 来自 uutils 内部的路径仍是 `\`：`ls -R` 的目录标题，`find -exec` 里的 `{}`。用户自己输入的 `\` 原样显示。
- PowerShell 的 Tab 补全仍给出 `\` 路径（计划放到 M3 的 shell 集成）。
- 通配符：`**` 按 glob 库递归匹配，bash 默认不递归（zsh 递归）；盘符相对写法 `C:*.txt` 会匹配到以点开头的名字。
- 带引号参数的标记：以 U+E000 开头的参数会被去掉这个字符。这个字符属于私用区，正常输入不会出现。
- uutils 的报错信息跟随系统语言（本机是中文）；`find` 在 Windows 上按系统格式输出，由 `ut` 转成 `/`；没有 `awk`、`rg`；`cmp` 说 "char" 而不是 "byte"；`cut -c` 按字节计。
- 只在 Windows 11（pwsh 7.6、Windows PowerShell 5.1、cmd）和 WSL Ubuntu 上测过。macOS 未运行。
- 实机检查在用户的真实 profile 下运行：pwsh 7 带 profile 和不带 profile 都测过，5.1 只测了不带 profile 的情况。

## 2026-11-17 · M1 终端基础

范围：`crates/uni-term`（按格定位的网格渲染、颜色与属性、选择、复制粘贴、回滚、鼠标上报、输入法文本通路、字体回退、连字开关、标签页、配置文件、`--replay`）、`tools/shot.ps1`、`tools/verify.ps1`、`tools/fetch-conpty.ps1`。

检查手段：单元测试 59 项；`tools/verify.ps1` 实测 30 项（启动真实窗口，向它自己的窗口句柄投递输入，回放它录下的输出流并比较截图像素）。

### 发现并修掉的问题（每次发现后干净轮数归零）

| # | 视角 | 问题 | 定性 | 处理 |
|---|---|---|---|---|
| 1 | 机制通路存活 | 连字从未生效。开和关的截图 0 像素差异。**我在 M0 记录里写"连字已渲染"是看错了截图。** | 结构缺陷：GPUI 的 Windows 排版把空的特性列表当成"关闭全部特性" | 始终显式写出 `calt` 的值。修后开关相差 563 像素，中文部分 0。全仓只有 1 处构造字体特性。 |
| 2 | 真实程序 | nvim 里含 emoji 或组合字符的行之后整屏错位。 | 结构缺陷：系统自带 ConPTY 会重绘输出，行间靠自动换行衔接，它的字符宽度算法和终端不一致 | 随程序附带新版 ConPTY 1.25.261109001（微软 MIT，NuGet 包，SHA-256 固定）。`portable-pty` 优先加载它。修后回放与 nvim 实际布局一致。 |
| 3 | 边界 | `-e 不存在的程序` 退出码是 0。配置里的 shell 不存在时窗口一闪而过。 | 边界遗漏 | 显式命令失败：退出码 1。配置的 shell 失败：改用默认 shell 并在标签页里说明。 |
| 4 | 静态一致性 | 响铃通知被产生但没人处理。 | 死代码 | 删除该通知。 |
| 5 | 代码正确性 | pty 输出队列无上限。 | 资源 | 改为有界队列（256 次读取），读线程会等待。 |
| 6 | 代码正确性 | 窗口没有可用空间时把屏幕缩成 2×1。 | 边界遗漏 | 放不下最小屏幕时保持原尺寸。 |
| 7 | 代码正确性 | 配置报错原文写上屏幕，可带控制字符。 | 边界遗漏 | 写入前去掉换行以外的控制字符。 |
| 8 | 代码正确性 | 输入法发来空文本也清选区并滚动。光标越界时排版层会 panic。 | 边界遗漏 | 空文本不处理；越界光标不画。 |
| 9 | 发布构建 | release 是窗口程序：在终端里 `--help` 不输出；stdout 管道关闭时 `print!` 触发 panic。 | 结构缺陷（Windows 子系统） | 启动时接入父控制台；stdout 写失败时丢弃而不 panic。 |
| 10 | 交互 | 鼠标上报模式下一格滚轮发了 3 次上报，nvim 滚 9 行。 | 边界遗漏 | 3 行滚轮合成 1 次上报。修后一格滚 3 行。 |
| 11 | 环境 | 从 keepane 窗格里启动时，shell 继承了 `KEEPANE_*`，keepane 拒绝嵌套。 | 结构缺陷 | 启动 shell 时去掉 `KEEPANE*`、`TMUX`、`TMUX_PANE`、`STY`。 |

检查工具自身的问题（不是产品问题，但同样记账）：全局 SendKeys 在窗口不在前台时会把按键发到别的程序，已改为向窗口句柄投递消息；多标签页共用一个日志导致回放互相覆盖，已改为每个会话一个文件；截图逐像素比较会因 GPU 混色舍入偶发差 1/255（11 次运行里 4 次，每次 2 个像素，输出流哈希完全相同），已加 2/255 容差。

### 连续三轮干净

| 轮 | 视角 | 构建 | 单元测试 | clippy | 实测 | 其他数字 |
|---|---|---|---|---|---|---|
| 1 | 机制通路存活 + 边界与退化输入 | release | 59/59 | 0 | 30/30 | 连字 563 px；回退字体 342 px；行高 4541 px；回滚上限 1325 px；2 万行输出全部到达；遗留进程 0 |
| 2 | 静态一致性 + 代码正确性 | debug | 59/59 | 0 | 30/30 | 14 个配置项全部被读取；8 个命令行参数全部被处理；非测试代码 `unwrap` 4（都是锁）、`expect` 1（开窗）、`panic!` 0、`unsafe` 1（带 SAFETY 说明）；遗留标记 0 |
| 3 | 可复现性 | release | 3 次都是 59/59 | 0 | 30/30 | 同一份录制流回放 6 次（debug 3 + release 3）哈希全部相同；设置类检查连跑两遍输出逐行相同 |

### 已知限制

- 输入法的"组合中"状态（拼音候选窗、预编辑文字）没有自动化验证。已验证的是提交文本的通路（`WM_CHAR`，中文字符能进 shell）。
- 带 Ctrl、Shift、Alt 的组合键在检查里通过共享键盘状态发送，没有按真实键盘。
- 只在 Windows 11 上实测。macOS 和 Linux 的代码路径已写，未运行。
- release 构建在 32 并行任务下 rustc 两次崩溃（`STATUS_ACCESS_VIOLATION`，崩在不同 crate 上），`-j 6` 下 3 次都成功。
- Shift-Enter 发送换行符（Claude Code 里是换行）。PowerShell 里它不是"续行"，要等实现 win32-input-mode 或 kitty 键盘协议。
- release 构建在终端里打印时不阻塞 shell，文字会出现在下一个提示符之后。

## 2026-11-16 · M0 技术验证（spike）+ 网页 demo

范围：`crates/uni-term`（GPUI 窗口 + pty + 屏幕文字 + 按键）、`demo/index.html`。

| 轮 | 视角 | 做了什么 | 结果 | 发现 |
|---|---|---|---|---|
| 1 | 代码正确性 | `cargo test`、`cargo clippy --all-targets` | 8/9 通过；clippy 1 条警告 | 测试 `the_end_of_the_program_is_reported` 不喂输出，ConPTY 的光标询问无人回答，子程序卡住。clippy：`bool::then` 的多余闭包。两处已修，本轮不计数。 |
| 2 | 机制通路存活 | 重跑测试和 clippy；启动窗口，用 SendKeys 输入 `echo`，截图 | 9/9 通过；clippy 0 条；窗口标题 `uni-term`，截图显示 PowerShell 7.6.10 提示符和回显，`-> => !=` 连字已渲染 | 干净。已知限制：SendKeys 发出的中文没有进入终端（需要 M1 接 `EntityInputHandler`）。 |
| 3 | 可复现性 | `cargo test` 连跑 3 次 | 3 次都是 9/9，耗时 0.39 s、0.41 s、0.41 s | 干净。 |

连续干净轮数：2（第 2、3 轮）。第三个干净轮没有做：用户要求先看方案和 demo，正式实现暂停。M1 开工时从 0 重新计数。

demo 页面（agent-browser 0.43.0，视口 1400×900）：

| 场景 | 结果 |
|---|---|
| 输入 `git` 后的候选列表，↓ 两次 | 6 条候选，选中 `git commit -m "wip"` |
| `cat data.json \| jq …`，`view photo.png` | 输出 3 行；1 个 canvas |
| 关闭"图形界面"后 `view`，`--mode braille` | 2 个字符画，状态栏显示"半块字符回退" |
| macOS + agent 侧栏 + `md README.md` | 1 个 markdown 块，4 个 agent 卡片 |
| `gti status`，Ctrl-E 解释 `tar …` | 纠正卡片 1 个，解释表 5 行 |
| 输入 `car`，按 → | 灰色建议 `go test -p uni-term`，接受后为 `cargo test -p uni-term` |
| Windows + 系统命令模式 | `jq` 报未找到；`which cat` 显示是 PowerShell 别名 |
| `view demo.mp4` | 600 ms 内同一像素由 `230,183,119` 变为 `149,108,121` |
| JS 报错 | 0 条 |

### 追加：解释和纠正改为弹窗（同日）

用户要求：解释窗口改为弹窗，回车后消失；纠正卡片同样改为弹窗。两者共用 `showPop`。

| 操作 | 弹窗数 | 滚动记录里的卡片 | 输入行 |
|---|---|---|---|
| 运行 `gti status` | 1（纠正） | 0 | 空 |
| Enter | 0 | 0 | `git status`，未运行 |
| 再按 Enter | 0 | 0 | 空，输出 `On branch main` |
| Esc | 0 | 0 | 空 |
| 输入 `l` | 0 | 0 | `l` |
| Ctrl-E | 0 | 0 | 空 |
| 点弹窗里的按钮 | 0 | 0 | `git status` |
| Ctrl-E 解释 `tar …`，再 Enter | 1 → 0 | 0 | 不变，未运行 |
| 运行 `nosuchcmd`（没有已知修正） | 0 | 0 | 空 |

终端写满时：纠正弹窗在 861–978 px，输入行在 982 px，可见区域 456–1016 px。JS 报错 0 条。

第一轮截图发现并修掉 4 处：低分辨率下星星变成白块；星星排成虚线；`tar` 的参数解释不准；`✗` 让提示符右移 3 px。
