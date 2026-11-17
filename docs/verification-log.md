# 验收记录

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
