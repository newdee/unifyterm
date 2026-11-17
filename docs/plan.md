# 实施计划：M3 到 M7

写于 2026-11-17。M0、M1、M2 已验收（见 `docs/verification-log.md`）。本文件规定剩下 5 个阶段各做什么、怎么设计、怎么验证。按步骤顺序执行。

| 阶段 | 内容 | 依赖 |
|---|---|---|
| M3 | shell 钩子、命令历史库、候选列表 | M2 |
| M4 | AI：建议、纠正、解释 | M3 |
| M5 | 图片（kitty graphics）、`ut view`、`ut md` | M1 |
| M6 | agent 侧栏、`ut mcp`、skill | M3 |
| M7 | 视频、安装包、三平台验证、发布 | 全部 |

下一期（通信功能、可观测平台）见 `docs/roadmap.md`，M7 之后再规划。

**挂账，不在当前顺序里：自己的 shell。** 用户 2026-11-17 决定先不做，等以后做自己的 shell 时再定路线。调研结果和验证方案保留在本文件末尾。在那之前，uni-term 只托管系统 shell（PowerShell、cmd、bash、zsh、fish）。

---

## 0. 每个阶段都要遵守的规则

### 0.1 固定流程

1. 读本文件里该阶段的一节。读 `~/.agents/skills/` 下的 BeautifulRust 规则。
2. 做该阶段的"第 0 步：先验证的事实"。任何一条不成立，先停下，改设计，把改动写回本文件。
3. 按步骤实现。每步完成后运行：
   ```
   cargo fmt --all
   cargo clippy --workspace --all-targets --all-features -j 8 -- -D warnings
   cargo test --workspace -j 8
   ```
4. 验收：连续三轮干净。每轮换一个视角，每轮都跑全量检查：
   ```
   pwsh tools/round.ps1 -Profile debug      # 第 1 轮：机制通路存活 + 边界输入
   pwsh tools/round.ps1 -Profile release    # 第 2 轮：静态一致性 + 代码正确性
   pwsh tools/round.ps1 -Profile debug      # 第 3 轮：可复现性（另加该阶段的重复运行检查）
   ```
   任何一轮发现问题：先记账，定性（个性还是共性；结构缺陷还是边界遗漏），评估回归，修掉，计数归零。
5. 在 `docs/verification-log.md` 顶部写该阶段的记录：发现的问题表、三轮的数字、已知限制。
6. 更新 `demo/index.html` 里计划表的对应行。
7. 提交。提交信息用英文，说明做了什么和为什么。
8. 直接进入下一阶段（用户 2026-11-17 改定：不再每个阶段停下等确认）。0.4 节里需要用户的事，记下来，最后一并报告。

### 0.2 代码规则

- Rust 按 BeautifulRust：无 `unwrap`（测试除外），无 `bool` 参数，用 `#[expect(lint, reason = "...")]` 而不是 `#[allow]`，`unsafe` 必须带 SAFETY 说明，二进制 crate 里用 `pub(crate)`。
- 每个新配置项必须同时满足三条：有代码读它；有实机检查证明改它会改变输出；写进帮助或文档。
- 每个新环境变量必须写进 `ut --help` 或 `uni-term --help`。
- 同一个值只能有一个来源。
- `ut` 自己生成的路径一律用 `slash::shown`。用户输入的路径原样显示。

### 0.3 测试规则

- 测试不得写入用户的真实数据。包括：PowerShell 历史文件、命令历史库、配置文件、剪贴板（用完恢复）。
- 实机检查看"敲入命令后的实际输出"，不看 `Get-Command` 这类间接证据。
- 期望值来自夹具、GNU 工具或 bash，不来自被测程序自己。
- 每个修复都要有"去掉修复后测试失败"的证明。
- 跨平台的输出用同一份 golden 文件逐字节比较。
- Linux 侧用 `wsl -- bash -l tools/linux-test.sh`。

### 0.4 不能自己做的事（需要用户）

- 推送到远程仓库、发布安装包、提交 winget 或 Homebrew。
- 修改用户的 profile、历史文件、全局配置。
- 改 keepane 仓库（本机的另一个仓库）：先说明改什么，等确认。
- macOS 实机验证：需要一台 Mac 或 CI。

---

## M3 shell 钩子、命令历史、候选列表

### 目标

终端知道提示符在哪、命令是什么、结果如何。命令存进本地库。输入时显示候选，可用 Ctrl-N/P 或方向键选择。

### 第 0 步：先验证的事实

1. `rusqlite`（0.43，`bundled` 特性）里 FTS5 和 `trigram` 分词器可用。写 5 行测试：`CREATE VIRTUAL TABLE t USING fts5(x, tokenize='trigram')`，插入，`MATCH` 查询。
2. alacritty_terminal 0.26 会忽略不认识的 OSC（133、633、7、9;9），不会把它们画到屏幕上。喂一段含这些序列的字节，检查屏幕文字。
3. PSReadLine 默认的 Escape 键清空当前行；bash、zsh、fish 里 Ctrl-E 加 Ctrl-U 清空当前行。逐个实测。

### 设计

#### 1. 数据目录可重定位（先做）

新环境变量 `UNI_TERM_DATA_DIR`。设置后，命令目录、启动脚本、历史库、日志都放在那里。所有实机检查都设置它，指向临时目录。这样测试不可能写到用户的真实数据。

#### 2. 终端标记协议

uni-term 识别这些标准序列（以 BEL 或 ST 结束）：

| 序列 | 含义 |
|---|---|
| `OSC 133;A` | 提示符开始 |
| `OSC 133;B` | 提示符结束，输入开始 |
| `OSC 133;C` | 命令开始执行 |
| `OSC 133;D;<退出码>` | 命令结束 |
| `OSC 7;file://host/path`、`OSC 9;9;<path>` | 当前目录 |
| `OSC 633;E;<命令行>` | 命令文本。转义：`\` 写成 `\\`，`;` 写成 `\x3b`，小于 0x20 的字节写成 `\xHH` |
| `OSC 633;P;Shell=<名字>` | shell 种类：`pwsh`、`bash`、`zsh`、`fish` |

用标准序列，不用私有序列：keepane 和其他工具也认识它们。

#### 3. 扫描器 `crates/uni-term/src/marks.rs`

- 一个字节流状态机。输入：pty 读到的一块字节。输出：若干段"普通字节"和"标记"。
- 普通字节原样交给 `Screen::feed`。遇到标记时，先把它之前的字节喂完，读取此刻的光标位置，再产生 `Mark { kind, line, column }`。
- 序列可以跨两次读取被切开。状态机必须保存未完成的部分。未完成的序列超过 64 KiB 时放弃并原样输出。
- M5 的 kitty graphics（APC 序列）复用同一个扫描器。

#### 4. 每个会话的提示符状态 `prompt.rs`

```
enum Phase { Unknown, Prompt, Input { start: (line, column) }, Running { command, started_at } }
```

- `B` 之后进入 `Input`。`C` 之后进入 `Running`。`D` 之后回到 `Unknown`，并产生一条记录。
- **当前输入的文本**：从屏幕网格读取，范围是 `start` 到光标位置。只读光标左边：右边可能是 shell 自己的灰色预测文字。
- **命令文本**：有 `633;E` 就用它；没有就用按下 Enter 那一刻读到的输入文本（cmd.exe 走这条路）。
- 进入备用屏幕（alt screen）时，状态置为 `Unknown`，候选列表关闭。

#### 5. 钩子的注入方式

只在用户没有给 shell 写自定义参数时注入（和 M2 的启动脚本规则相同）。

| shell | 注入方式 |
|---|---|
| pwsh、powershell | `start.ps1` 新增一段：包住现有的 `prompt` 函数，前后输出标记。命令文本取 `(Get-History -Count 1).CommandLine`，退出码取 `$?` 和 `$LASTEXITCODE`。现有的"默认提示符显示 `/`"逻辑保留在被包住的那一层。 |
| bash | `bash --rcfile <文件>`。文件先 source `~/.bashrc`，再加载钩子：`PS1` 前后加 A、B；`PS0` 输出 C；`PROMPT_COMMAND` 输出 D、目录和 `633;E`。 |
| zsh | 设置 `ZDOTDIR` 指向我们的目录。那里的 `.zshrc` 先 source 用户原来的，再用 `add-zsh-hook precmd/preexec`。 |
| fish | `fish --init-command 'source <文件>'`，用 `fish_prompt`、`fish_preexec`、`fish_postexec` 事件。 |
| cmd.exe | 设置环境变量 `PROMPT=$E]133;A$E\$P$G$E]133;B$E\`。只有 A、B。命令文本从网格读，退出码未知。 |

钩子脚本放在 `crates/uni-term/src/hooks/`，用 `include_str!` 嵌入，启动时写到数据目录。做法参考 keepane 的 `src/shellhook.rs` 和 `src/platform/windows/shell.rs`（同一作者，MIT），文件头注明来源。

#### 6. 历史库：新 crate `crates/history`（库，名字 `uni-history`）

GUI、`ut history`、M6 的 `ut mcp` 共用它。文件：`<数据目录>/history.db`。

```sql
PRAGMA journal_mode = WAL;
CREATE TABLE commands (
  id         INTEGER PRIMARY KEY,
  text       TEXT NOT NULL UNIQUE,
  count      INTEGER NOT NULL,
  ok_count   INTEGER NOT NULL,
  fail_count INTEGER NOT NULL,
  last_at    INTEGER NOT NULL          -- Unix 毫秒
);
CREATE TABLE runs (
  id          INTEGER PRIMARY KEY,
  command_id  INTEGER NOT NULL REFERENCES commands(id),
  at          INTEGER NOT NULL,
  cwd         TEXT NOT NULL,            -- 一律用 /
  exit        INTEGER,                  -- 未知为 NULL
  duration_ms INTEGER,
  shell       TEXT NOT NULL
);
CREATE INDEX runs_by_command ON runs(command_id, at);
CREATE INDEX runs_by_cwd ON runs(cwd, at);
CREATE VIRTUAL TABLE commands_fts USING fts5(text, content='commands', content_rowid='id', tokenize='trigram');
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);   -- schema 版本
```

接口：

```rust
pub struct History { /* 一个连接 */ }
impl History {
    pub fn open(path: &Path) -> Result<Self>;
    pub fn record(&mut self, run: &Run) -> Result<Recorded>;      // Recorded::Stored 或 Recorded::Refused(原因)
    pub fn search(&self, query: &Query) -> Result<Vec<Candidate>>;
    pub fn recent(&self, limit: usize, cwd: Option<&str>) -> Result<Vec<Candidate>>;
}
pub struct Query<'a> { pub text: &'a str, pub cwd: &'a str, pub now_ms: i64, pub limit: usize }
```

`now_ms` 由调用方传入：排序因此可以用固定时间做确定性测试。

**召回**：
- 查询为空：最近的命令，当前目录的优先。
- 1 到 2 个字符：前缀查询 `text >= ?1 AND text < ?2`（走 UNIQUE 索引）。
- 3 个字符以上：FTS5 trigram `MATCH`（子串匹配），最多取 200 行。

**排序**（在 Rust 里对召回的行计算）：

```
分数 = 4.0 × 是前缀匹配
     + 1.0 × ln(1 + count)
     + 2.0 × exp(−距上次使用的天数 / 7)
     + 1.5 × 在当前目录运行过
     − 2.0 × fail_count / count
```

分数相同时：`last_at` 新的在前，再按文本排序。权重是常量，集中在一处，带名字。

**不入库的命令**：
- 以空格开头的（bash 的惯例）。
- 匹配密钥模式的：`password|passwd|secret|token|api[_-]?key|authorization|bearer` 后跟 `=` 或 `:` 或空格加值；`--password`；`AKIA[0-9A-Z]{16}`；`ghp_`、`gho_`、`sk-`、`xox[bap]-` 开头的长串；`://用户:密码@` 形式的 URL。
- 超过 4096 字节的。

模式集中在 `secrets.rs`，M4 的脱敏复用它。

**线程**：库的读写在一个专用线程上。GUI 发请求，带递增的序号；结果回来时序号不是最新的就丢弃。GUI 线程不等库。

#### 7. 候选列表（GUI）

- 出现条件：会话处于 `Input`；不在备用屏幕；输入文本非空且有结果。
- 位置：光标所在行的下方。下方不够 3 行时放上方。最多 8 行。
- 每行：命令文本（匹配部分高亮），右侧显示次数和上次使用的相对时间。
- 按键（只在列表可见时截获）：

| 键 | 动作 |
|---|---|
| Ctrl-N、↓ | 选下一条。初始没有选中 |
| Ctrl-P、↑ | 选上一条 |
| Tab、→（有选中时） | 把选中的命令填进输入行，不运行 |
| Enter（有选中时） | 填进输入行，不运行。再按一次 Enter 才运行 |
| Enter（没有选中时） | 交给 shell |
| Esc | 关闭列表。本次输入不再弹出，直到输入内容变化 |

- **填入输入行的做法**：先清空 shell 的当前行，再发送文本。清空的按键按 shell 种类选：`pwsh` 发 Esc；其他发 Ctrl-E 加 Ctrl-U。shell 种类来自 `633;P;Shell=`；未知时不提供填入功能，只显示列表。文本用括号粘贴模式发送（shell 开启了的话）。
- 配置：`[history] enabled = true`、`candidates = true`、`limit = 8`。

#### 8. `ut history`

`ut history search <文本>`、`ut history recent [-n N]`、`ut history forget <文本>`（删除一条命令的全部记录）。只读操作用只读连接。

#### 9. keepane 里的候选列表（可选，需要用户确认）

在 keepane 里运行时，标记被 keepane 消费。需要改 keepane：新选项把活动窗格的 A、B、C、D、目录和 `633;E` 转发给外层终端，并在转发 `B` 之前把真实光标移到对应位置。这要改 wmux 仓库。先写设计，等用户确认再做。它不是 M3 验收的条件。

### 步骤

1. `UNI_TERM_DATA_DIR`：实现、文档、`tools/shot.ps1` 和 `tools/verify.ps1` 全部使用它。跑一遍 M2 的 59 项，确认不变。
1b. `ut` 进入命令目录（M2 的已知缺口）：`ut --links` 额外创建 `ut` 自己的链接，终端里才能敲 `ut history`、`ut view`、`ut md`。更新 `links.rs` 的测试；实机检查加一条"敲 `ut --version` 有输出"。
2. `marks.rs`：扫描器和单元测试（见"验证"）。
3. `session.rs`：读线程先过扫描器；`Notice` 增加 `Mark`。
4. `prompt.rs`：状态机和单元测试。
5. `crates/history`：建库、迁移、`record`、`search`、`recent`、`secrets.rs`、测试。
6. 钩子脚本 5 份和注入逻辑。`shell.rs` 按 shell 种类选择。
7. 记录通路：`D` 到达时，把 `Run` 发给历史线程。
8. 候选列表的查询通路和界面。
9. 填入输入行。
10. `ut history`。
11. `tools/verify.ps1` 加一节 `history`。
12. `demo/index.html`：候选列表的按键说明与本节一致。

### 验证

**单元测试**
- 扫描器：每种序列各 1 条；BEL 和 ST 两种结尾；一个序列被切成 2 块、3 块、每字节 1 块，结果相同；标记前后的普通字节一个不少；超长未完成序列原样输出；1 MiB 随机字节不崩溃且输出等于输入。
- 状态机：A→B→C→D 的正常顺序；缺 C；连续两个 A；进入备用屏幕；`D` 没有退出码。
- 历史库：记录后能查到；同一命令记两次 `count = 2`；失败次数；前缀查询；子串查询；空查询；目录优先。
- 排序：用固定 `now_ms` 和手工构造的 6 条命令，期望顺序写死在测试里。
- 密钥：100 行测试集（50 行含密钥，50 行不含）。含密钥的 50 行全部拒绝；不含的 50 行全部接受。

**性能测试**（release，`cargo test --release -p uni-history -- --ignored`）
- 生成 10 万条不同的命令。跑 1000 次查询（长度 1 到 12 的混合）。p95 低于 5 ms。
- 单条 `record` 平均低于 2 ms。
- 把 p50、p95、p99 写进验收记录。

**实机检查**（`tools/verify.ps1 -Section history`，数据目录用临时目录）
1. 运行 3 条命令后，`ut history recent` 列出这 3 条，顺序正确。
2. 一条失败的命令（`ls no-such-dir`）：库里 `exit` 非 0。
3. 换目录后运行命令：库里 `cwd` 是新目录，用 `/`。
4. 敲入一条带 `--password x` 的命令：库里没有它。
5. 输入 `ec`：候选列表出现（截图里列表区域的像素与无列表时不同），其中有之前运行过的 `echo ...`。
6. Ctrl-N 再 Tab：输入行变成选中的命令；命令没有运行（输出里没有它的结果）。
7. 再按 Enter：命令运行。
8. Esc：列表消失。
9. 在 nvim（备用屏幕）里输入：没有列表。
10. `[history] enabled = false`：库文件不增长，没有列表。
11. bash（WSL 或 Git Bash，机器上有的话）和 cmd.exe 各做第 1、5 项。
12. 用户的真实历史库不存在或字节数不变。

**可复现性**：同一组命令在两个新的临时数据目录里各跑一遍，`ut history recent` 的输出（去掉时间）相同。

### 验收标准

1. 10 万条历史下，单次查询 p95 低于 5 ms。
2. 失败过的命令排在同等条件下成功的命令之后（排序测试）。
3. 密钥测试集 100% 命中，0 误伤。
4. pwsh 7、Windows PowerShell 5.1、cmd 三种 shell 里候选列表都能出现。bash、zsh、fish 在 WSL 里验证钩子输出的标记（uni-term 的 Linux 界面要到 M7 才验证）。
5. 关闭 `history.enabled` 后，行为与 M2 相同。

### 风险

- shell 的行编辑器把输入画在别处（右提示符、多行编辑）。对策：只读 `B` 到光标的范围；多行输入时列表不出现。
- 清空当前行的按键被用户改过。对策：填入功能可用配置关闭；列表仍可用鼠标点击复制。

---

## M4 AI：建议、纠正、解释

### 目标

输入时给出灰色建议；命令失败后弹出修正；按键唤出对命令每个部分的解释。没有网络和后端时，终端其他功能不受影响。默认不启用任何后端。

### 决定变更（需要同步到 demo）

demo 里解释的快捷键是 Ctrl-E。它和 bash、zsh、fish 的"移到行尾"冲突。改为 **Ctrl-Shift-E**（和 Ctrl-Shift-T/W/C/V 同一套）。第 1 步先改 demo。

### 第 0 步：先验证的事实

1. 读 `claude-api` skill，确认 Anthropic Messages API 的请求格式、模型 ID 和结构化输出的做法。
2. `ureq` 3.x 能在 GPUI 的后台执行器里做阻塞请求，并支持超时。
3. 用一个本地假服务（测试里起在 127.0.0.1）能完整走通请求和响应。

### 设计

#### 1. 新 crate `crates/assist`（库）

```rust
pub trait Provider: Send + Sync {
    fn complete(&self, request: &Request, timeout: Duration) -> Result<String, ProviderError>;
}
pub struct Request { pub system: String, pub user: String, pub max_tokens: u32 }
```

两个实现：`Anthropic`（Messages API）和 `OpenAiCompatible`（`/v1/chat/completions`，任意 `base_url`）。测试用第三个：`Fixed`（返回写死的文本）。

三个功能都是"拼请求 → 调 `Provider` → 解析 JSON"：

| 功能 | 输入 | 输出（JSON） | 超时 |
|---|---|---|---|
| `suggest` | 已输入的文本、当前目录、最近 5 条命令 | `{"completion": "..."}`，必须以已输入文本开头 | 3 s |
| `fix` | 失败的命令、退出码、输出的最后 40 行 | `{"command": "...", "reason": "..."}` | 15 s |
| `explain` | 命令文本、（`ut` 的命令时）它的 `--help` | `{"parts": [{"text": "...", "meaning": "..."}]}`，各部分拼起来等于原命令 | 20 s |

输出不符合格式或不满足约束：丢弃，不显示，写日志。

#### 2. 先本地，后模型

- **纠正**：先用本地规则。命令名不存在时，在 PATH 里的命令名和历史库里找编辑距离不超过 2 的（`gti` → `git`）。本地有结果就不问模型。没有配置后端时，只有本地规则。
- **解释**：`ut` 的 99 个命令先取自己的 `--help`，作为上下文交给模型。不运行别的程序的 `--help`（会有副作用）。没有配置后端时：显示"未配置后端"和配置方法。
- **建议**：只有模型。没有后端时不出现。历史候选列表（M3）始终可用。

#### 3. 脱敏（发送前）

复用 `uni-history` 的 `secrets.rs`。命中的片段替换为 `‹redacted›`。另外：用户主目录替换为 `~`。脱敏在 `assist` 的入口统一做，三个功能都经过它。

#### 4. 配置

```toml
[ai]
provider = "none"            # none | anthropic | openai-compatible
model = ""                   # 例如 claude-haiku-5-5（建议用，延迟低）或 claude-sonnet-5-5
base_url = ""                # openai-compatible 时必填
api_key_env = ""             # 存放密钥的环境变量名。密钥不写进配置文件
suggest = true
fix = true
explain = true
```

第一次触发任何 AI 功能时，弹窗说明哪些文本会被发送，用户确认后才发送。确认结果存进数据目录。

#### 5. 界面

- **建议**：光标后的灰色文字。→ 接受。继续输入则重新请求（防抖 300 ms，序号过期的结果丢弃）。候选列表可见时不显示建议。
- **纠正弹窗**：命令结束且退出码非 0，并且判定为"命令不存在"（退出码 127，或 PowerShell 的 CommandNotFound 文本）时出现在输入行旁。Enter 把修正填进输入行，不运行。Esc 关闭。
- **解释弹窗**：Ctrl-Shift-E。Enter 或 Esc 关闭。不运行命令。
- 弹窗用 M3 候选列表的同一个组件。

### 步骤

1. 改 demo 的快捷键和说明。
2. `crates/assist`：`Provider`、两个实现、`Fixed`、三个功能、脱敏入口、单元测试。
3. 本地纠正规则和测试。
4. 配置项和校验（`provider` 不是 `none` 时 `model` 和 `api_key_env` 必填）。
5. 首次使用的确认弹窗。
6. 建议的界面和请求通路。
7. 纠正弹窗和解释弹窗。
8. `tools/verify.ps1` 加一节 `ai`，用本地假服务。

### 验证

**单元测试**
- 三个功能各有：正常响应；格式错误的 JSON；违反约束的响应（建议不以输入开头；解释的各部分拼不回原命令）；超时；HTTP 500。
- 脱敏：M3 的 100 行测试集，含密钥的 50 行发送出去的文本里一个密钥都没有（逐条断言）。
- 本地纠正：20 个拼错的命令名，期望的修正写死。

**实机检查**（假服务记录它收到的每个请求）
1. `provider = "none"`：M2、M3 的全部检查通过；假服务收到 0 个请求。
2. 配了假服务后输入 `car`：出现灰色建议；按 → 后输入行变成完整命令。
3. 运行 `gti status`：出现纠正弹窗；Enter 后输入行是 `git status`，命令没有运行。
4. Ctrl-Shift-E：出现解释弹窗；Enter 后消失，输入行不变。
5. 输入含 `--token abc123` 的命令并触发建议：假服务收到的请求体里没有 `abc123`。
6. 假服务延迟 30 秒：终端仍可输入，超时后无弹窗，日志里有一条。
7. 首次使用：先出现确认弹窗；拒绝后假服务收到 0 个请求。

**真实后端**：用用户提供的密钥手工验证一次三个功能（需要用户）。结果写进验收记录。

### 验收标准

1. 无网络、无后端时，终端全部功能照常，AI 功能安静地不可用。
2. 脱敏测试集 100% 命中。
3. 弹窗里的 Enter 从不运行命令。
4. 用户确认之前，没有任何文本发送出去。

---

## M5 图片、`ut view`、`ut md`

### 目标

uni-term 窗口里能显示图片（kitty graphics 协议）。`ut view` 在任何终端里选对显示方式。`ut md` 渲染 markdown。

### 第 0 步：先验证的事实

1. GPUI 0.2.2 画位图的接口：`RenderImage`、`window.paint_image` 的确切签名。写一个把 2×2 像素图画到网格上的最小例子。
2. alacritty_terminal 的解析器忽略 APC 序列（`ESC _ ... ESC \`），不画到屏幕上。
3. `image` 0.25 解码 PNG、JPEG、GIF、WebP。

### 设计

#### 1. 协议子集（`crates/uni-term/src/graphics.rs`）

扫描器（M3 的 `marks.rs`）增加 APC `G` 序列。支持：

| 键 | 支持的值 |
|---|---|
| `a` | `t` 传输、`T` 传输并显示、`p` 显示、`d` 删除、`q` 查询 |
| `f` | `24`、`32`、`100`（PNG） |
| `t` | `d`（直接传输，base64）。分块 `m=1` |
| `c`、`r` | 占多少列、多少行 |
| `i`、`p` | 图片编号、放置编号 |
| `d` | `a`（全部）、`i`（按编号） |
| `q` | `1`、`2`（抑制回应） |

`a=q` 要回应 `OK`：工具靠它探测终端是否支持。不支持的键：按协议回应错误，不崩溃。文件传输（`t=f`）、共享内存、动画帧不做。

#### 2. 图片怎样跟着文字滚动

不另外记录"图片在第几行"。放置图片时，终端自己往网格的对应格子里写**占位字符**（kitty 的 Unicode 占位符 U+10EEEE，图片编号放在前景色里，行列用附加符号编码）。

- 滚动、回滚、清屏都由网格处理，图片自然跟着走。
- 程序自己输出占位符（kitty 的"虚拟放置"，keepane 和 tmux 穿透图片要用它）走同一条路。
- 一个机制覆盖两种用法。

`frame.rs`：遇到占位格子，产生 `ImageTile { image, source_rect, cell }`。`terminal.rs` 用 `paint_image` 画。

#### 3. 资源上限

- 单张图片解码后不超过 64 MiB。
- 全部图片合计不超过 256 MiB。超出时删除最久没有被任何格子引用的。
- 没有任何格子引用的图片在下一次清理时删除。
- 上限是具名常量。

#### 4. `ut view FILE`（`crates/ut/src/view/`）

选择显示方式，按顺序取第一个可用的：

| 顺序 | 方式 | 判断 |
|---|---|---|
| 1 | kitty graphics | `TERM_PROGRAM=uni-term`，或发 `a=q` 后 100 ms 内收到 `OK` |
| 2 | iTerm2 内联图片 | `TERM_PROGRAM` 是 `iTerm.app` 或 `WezTerm` |
| 3 | sixel | 设备属性回应里有 `4` |
| 4 | 半块字符 `▀` + 真彩色 | `COLORTERM=truecolor` |
| 5 | 盲文点阵 | 终端是 UTF-8 |
| 6 | ASCII | 其他情况；`NO_COLOR`；`TERM=dumb` |

- `--mode kitty|iterm|sixel|half|braille|ascii` 强制指定。`--width N` 指定列数。
- stdout 不是终端时：默认用 `ascii`，宽 80 列。结果是确定的，可以进 golden。
- 探测时要把终端设成原始模式并在退出时恢复，包括被 Ctrl-C 中断时。
- 缩放：保持宽高比。一个字符格按宽 1 高 2 计。

#### 5. `ut md FILE`（`crates/ut/src/md.rs`）

`pulldown-cmark` 解析，输出带 ANSI 样式的文本。

| 元素 | 渲染 |
|---|---|
| 标题 | 加粗，一级标题加下划线，前后空行 |
| 强调、加粗、删除线、行内代码 | 对应的 SGR 属性 |
| 列表、任务列表 | `•`、`1.`、`☐`/`☑`，按层级缩进 |
| 代码块 | 缩进 2 格，暗色。v1 不做语法高亮 |
| 引用 | 左侧 `│` |
| 表格 | 制表符画边框，按显示宽度对齐（CJK 占 2 列） |
| 链接 | OSC 8 超链接，文本后跟暗色的 URL |
| 图片 | 调用 `view` 的同一套逻辑；不能显示图片时写 `[图片：alt]` |
| 分隔线 | 一行 `─` |

- 按终端宽度折行；`--width N` 指定。stdout 不是终端时宽 80、无颜色（除非 `--color=always`）。
- GUI 里的 AI 弹窗（M4）用同一个解析器，但直接画成 GPUI 元素。这部分在 M5 里补上。

### 步骤

1. 第 0 步的三项验证。
2. 扫描器增加 APC；`graphics.rs` 解析控制键和分块传输。
3. 图片库：解码、编号、上限、清理。
4. 占位字符的写入和 `frame.rs`、`terminal.rs` 的绘制。
5. `a=q` 和各种错误的回应。
6. `ut view`：6 种输出方式、探测、`--mode`、`--width`。
7. `ut md`。
8. 测试夹具：一张 64×64 的 PNG（4 个色块加一条对角线），一个覆盖全部元素的 markdown 文件。
9. `tools/verify.ps1` 加一节 `graphics`。

### 验证

**单元测试**
- 协议解析：每个支持的键；分 1 块、3 块传输同一张图，结果相同；base64 错误；尺寸与数据不符；未知的键。
- 图片库：超过单张上限被拒绝；超过合计上限时删除的是最久未引用的；删除后内存计数归零。
- `ut view`：夹具图片在 `half`、`braille`、`ascii` 三种方式、宽 40 列下的输出与 golden 逐字节相同，Windows 和 Linux 共用 golden。
- `ut md`：夹具 markdown 在宽 80、无颜色和 `--color=always` 下的输出与 golden 相同。CJK 表格对齐单独一条。

**实机检查**
1. `ut view fixture.png`：窗口里图片区域的像素与夹具的 4 个色块颜色一致（各取一个采样点，容差 2/255）。
2. 输出 40 行文字后：图片向上移动了 40 行的高度（比较前后截图）。
3. `clear`：图片消失，采样点变回背景色。
4. 改窗口大小：不崩溃，图片仍在它的文字位置。
5. 连续显示 100 张大图：进程内存不超过上限加 100 MiB（读进程工作集）。
6. `ut view --mode half fixture.png`：屏幕文字里有 `▀`。
7. `ut view fixture.png | cat`：输出是 ASCII，没有转义序列。
8. `ut md fixture.md`：标题、表格边框、列表符号出现在屏幕文字里。
9. 在 Windows Terminal 里运行 `ut view`（它支持 sixel）：自动选了 sixel（`--verbose` 打印所选方式）。机器上没有就记为未测。
10. nvim 的 image.nvim：需要安装插件，作为手工检查，结果写进记录。

**可复现性**：同一张图显示 3 次，截图的图片区域 0 像素差异（容差 2/255）。

### 验收标准

1. 同一张图在 uni-term、不支持图片的终端、管道里各自选对了方式。
2. 图片跟随文字滚动，清屏后消失。
3. 内存有上限，并且测过。
4. `ut view`、`ut md` 的文本输出在 Windows 和 Linux 上逐字节一致。

### 风险

- GPUI 0.2.2 画位图的接口与预期不同。对策：第 0 步先验证；不行就把图片切成纹理图集。
- 占位字符在不认识它的字体里显示成方块。对策：`frame.rs` 把占位格子从文字绘制里排除。

---

## M6 agent 侧栏、`ut mcp`、skill

### 目标

在 uni-term 里看到 keepane 管理的 agent。让外部 agent 通过 MCP 查询命令历史和命令帮助。给 agent 一份 skill，说明在 uni-term 里怎样工作。

### 第 0 步：先验证的事实

1. 在一个测试用的 keepane 服务（`keepane -L uni-term-test`）里起两个窗格，运行 `keepane -L uni-term-test list-agents -J`，把输出存为夹具 `crates/uni-term/tests/fixtures/agents.json`。记录字段名。**不要碰默认 socket 上用户的会话。**
2. 确认 keepane 有没有"切换到某个窗格"的命令，以及它的确切写法。
3. 读 keepane 的 `src/mcp.rs`：它支持的协议版本是 `2025-06-18`、`2025-03-26`、`2024-11-05`。`ut mcp` 用同样的列表。

### 设计

#### 1. agent 侧栏

- 开关：Ctrl-Shift-A。配置 `[agents] sidebar = false`（启动时是否显示）、`poll_ms = 2000`。
- 数据：侧栏可见时，每 `poll_ms` 运行一次 `keepane list-agents -J`（无窗口启动，超时 1 秒）。不可见时不运行。
- 每个 agent 一张卡片：名字、状态、所在窗格、当前工作目录。字段以第 0 步的夹具为准。
- 点击卡片：如果当前标签页运行的是 keepane，就让 keepane 切到那个窗格；否则把窗格地址复制到剪贴板。
- keepane 不存在、没有服务在跑、输出不是合法 JSON：侧栏显示一句说明，不弹错误，不写满日志（同一种错误 1 分钟最多记 1 次）。
- 解析用 serde，未知字段忽略，缺少可选字段不报错。

#### 2. `ut mcp`（`crates/ut/src/mcp.rs`）

stdin/stdout 上的 JSON-RPC，每行一条。手写，不引入 MCP SDK（做法同 keepane，约 300 行）。

工具：

| 名字 | 参数 | 返回 |
|---|---|---|
| `history_search` | `query`，可选 `cwd`、`limit`（默认 20，最大 100） | 命令、次数、上次使用时间、上次退出码 |
| `history_recent` | 可选 `cwd`、`limit` | 同上 |
| `list_commands` | 无 | `ut` 的命令名 |
| `command_help` | `name` | 该命令的 `--help` 文本 |

- 历史库用只读连接打开。库不存在时返回空列表，不创建文件。
- 不提供"运行命令"的工具：agent 自己有 shell。
- 不提供"读屏幕"的工具：那需要和 GUI 进程通信，留到下一期的通信功能。
- 方法：`initialize`、`notifications/initialized`、`tools/list`、`tools/call`、`ping`。未知方法返回 `-32601`。非法 JSON 返回 `-32700`。

#### 3. skill 文件

`skills/uni-term/SKILL.md`。内容：
- 在 uni-term 里，Windows 上也能直接用 `cat`、`grep`、`jq`、`find`、`sed` 等 99 个命令。
- 路径用 `/`。通配符按 Unix 规则展开，带引号的不展开。
- 怎样注册 MCP：`claude mcp add uni-term -- ut mcp`。
- 四个工具各自什么时候用。
- 不要做的事：不要假设 `awk`、`rg` 存在。

`ut skill` 打印这份文件，`ut skill --install <目录>` 把它写到指定目录。不自动写入任何 agent 的配置目录。

### 步骤

1. 第 0 步：夹具和命令确认。
2. `agents.rs`：数据结构、解析、轮询。单元测试用夹具。
3. 侧栏界面和快捷键。
4. `ut mcp`：协议层、四个工具、测试。
5. skill 文件、`ut skill`。
6. `tools/verify.ps1` 加一节 `agents`。测试用 `keepane -L uni-term-test`，结束时只关这个服务。

### 验证

**单元测试**
- agent 解析：夹具；空列表；多一个未知字段；少一个可选字段；不是 JSON。
- MCP：`initialize` 带已知版本返回同一版本，带未知版本返回最新版本；`tools/list` 返回 4 个工具且每个都有 `inputSchema`；四个工具各一条正常调用；缺必填参数；未知工具；未知方法；非法 JSON；`limit` 超过 100 时取 100。
- `command_help`：对 99 个命令每个都返回非空文本。

**实机检查**
1. 测试用 keepane 服务里有 2 个窗格：打开侧栏后屏幕上出现 2 张卡片，名字与夹具一致。
2. 关掉测试服务：2 秒内侧栏显示"没有 keepane 服务"，没有错误弹窗。
3. PATH 里没有 keepane（用临时 PATH）：侧栏显示"未找到 keepane"。
4. 侧栏关闭时：10 秒内没有新的 `keepane` 进程被启动（数进程创建次数）。
5. 脚本当 MCP 客户端：`initialize` → `tools/list` → `history_search`，查到 M3 检查里写入的命令。
6. 历史库不存在时调用 `history_recent`：返回空列表，库文件没有被创建。

**真实 agent**：用 Claude Code 注册 `ut mcp` 并让它查一次历史。这会改用户的 Claude Code 配置，需要用户确认后做。结果写进记录。

### 验收标准

1. 一个真实的 agent 通过 MCP 查询历史并得到正确结果。
2. keepane 不存在时，uni-term 的其他功能不受影响。
3. 侧栏不可见时，不产生任何后台进程。

---

## M7 视频、安装包、三平台验证、发布

### 目标

终端里能播放视频。三个平台各有一个可安装的包。同一份配置和历史文件在三个平台可用。

### 第 0 步：先验证的事实

1. 机器上的 `ffmpeg` 能把视频解成原始 RGB 帧输出到管道：`ffmpeg -i f.mp4 -f rawvideo -pix_fmt rgb24 -`。
2. WSLg 里能否运行 uni-term 的 Linux 版（GPUI 需要 Vulkan）。不能就用 CI 加软件渲染（lavapipe、xvfb）。
3. `cargo-wix`、`cargo-deb` 的当前用法。
4. 用 `cargo-about`（或 `cargo-license`）生成全部依赖的许可证清单，确认没有与 MIT 发布不兼容的许可证。

### 设计

#### 1. 视频：`ut view FILE`（文件是视频时）

- 用 `ffprobe` 取宽、高、帧率、时长。用 `ffmpeg` 把帧解到管道，缩放到目标大小，帧率最高 30。
- 支持图片的终端：每帧用 kitty graphics 以同一个图片编号替换。其他终端：每帧用半块字符，光标回到起点重画。
- 按帧的时间戳定速。解码跟不上时丢帧，不拖慢播放。
- `q` 或 Ctrl-C 停止。停止时恢复光标和终端模式，结束 ffmpeg 进程。
- 没有声音（v1）。帮助里写明。
- 找不到 `ffmpeg`：打印一句说明和安装方法，退出码 1。不捆绑 ffmpeg（许可证和体积）。

#### 2. 版本和许可证

- 版本 0.1.0。`LICENSE`（MIT）。`THIRD-PARTY-NOTICES`：uutils、jaq、ripgrep 的库、GPUI、alacritty_terminal、微软 ConPTY 等的许可证全文。
- `uni-term --version`、`ut --version` 显示版本和 git 提交。

#### 3. 安装包

| 平台 | 包 | 内容 | 工具 |
|---|---|---|---|
| Windows | MSI | `uni-term.exe`、`ut.exe`、`conpty.dll`、`OpenConsole.exe`、开始菜单项 | `cargo-wix` |
| Windows | winget 清单 | 指向 MSI 的 YAML | 手写 |
| macOS | `.app` 加 `.dmg` | 两个程序、`Info.plist`、图标 | 脚本 |
| macOS | Homebrew cask | 指向 `.dmg` | 手写 |
| Linux | `.deb` | 两个程序、`.desktop` 文件、图标 | `cargo-deb` |
| Linux | AppImage | 同上 | `linuxdeploy` |

需要用户的：macOS 签名和公证（Apple 开发者账号）；winget、Homebrew 的提交；GitHub Release。没有签名时，包仍然能构建，文档里写明首次打开的步骤。

#### 4. CI（`.github/workflows/ci.yml`）

三个平台的矩阵：`windows-latest`、`macos-14`、`ubuntu-24.04`。每个平台：
1. `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test --workspace`。
2. 构建 release。
3. 运行 84 条命令记录（三个平台用同一份 golden）。
4. GUI 冒烟测试：启动 uni-term，运行 `echo ok` 后退出，用 `--replay` 检查录下的输出里有 `ok`。Linux 用 xvfb 加 lavapipe。
5. 打包，上传构建产物。

推送和启用 CI 需要用户的远程仓库。

#### 5. 跨平台的配置和历史

- 配置文件：三个平台同一格式。路径值里用 `/`。与平台有关的项（默认 shell、字体）在缺省时按平台取默认值。
- 历史库：`cwd` 一律存 `/` 形式。把 Windows 上生成的库拷到 Linux 上能打开、能查询。

### 步骤

1. 第 0 步四项。
2. 视频播放和测试夹具（一段 2 秒、64×64、每秒变一次颜色的视频，用 ffmpeg 生成，不入库）。
3. 版本信息、`LICENSE`、`THIRD-PARTY-NOTICES`。
4. Windows：MSI 和 winget 清单。在一台干净的环境（Windows Sandbox 或新用户）里安装、启动、卸载。
5. Linux：`.deb` 和 AppImage。在 WSL 的干净发行版里安装。
6. macOS：`.app` 的构建脚本和 cask。只能在 CI 上构建和冒烟。
7. CI 工作流文件。本地先用 `act` 或逐条命令验证能跑。
8. 用户文档 `README.md`：安装、配置项全表、快捷键全表、环境变量全表、已知限制。
9. 发布清单 `docs/release.md`。
10. 向用户报告，等确认后才推送、打标签、发布。

### 验证

**单元测试**
- 帧定速：给定时间戳序列和"解码耗时"序列，输出显示哪些帧、丢哪些帧，期望写死。
- 版本字符串的格式。

**实机检查**
1. 播放夹具视频：0.5 秒和 1.5 秒时各截一张图，同一像素的颜色不同，且各自等于夹具里那一秒的颜色。
2. 播放中按 `q`：1 秒内回到提示符，光标可见，没有 `ffmpeg` 进程残留。
3. PATH 里没有 ffmpeg：打印说明，退出码 1。
4. MSI：安装后开始菜单有 uni-term；启动后 `cat --version` 可用；卸载后安装目录不存在。用户数据目录保留（文档里写明）。
5. `.deb`：`dpkg -i` 成功；`uni-term --version` 和 `ut --list` 可用；`dpkg -r` 成功。
6. 把 Windows 上的 `config.toml` 和 `history.db` 拷到 Linux：`ut history recent` 列出同样的命令；配置被接受。
7. 全部之前阶段的检查在 release 版安装包里的程序上再跑一遍。

**CI**：三个平台全绿。macOS 的 GUI 冒烟结果写进记录；失败时写明原因，并记为"macOS 未验证"。

**文档一致性**：`README.md` 里的配置项、快捷键、环境变量，用脚本和代码逐项对照，数量和名字都相同。

### 验收标准

1. 三个平台各有一个可安装的包，安装后能启动并运行命令。
2. 同一份配置和历史文件在三个平台可用。
3. 许可证清单完整，没有不兼容的许可证。
4. 文档里的每个配置项、快捷键、环境变量都与代码一致。
5. 用户确认后完成发布。

### 风险

- macOS 无法实机验证。对策：CI 构建加冒烟；在发布说明里写明验证程度。
- GPUI 0.2.2 在 crates.io 上自 2025-10 起没有更新。对策：Linux、macOS 上遇到 GPUI 的问题时，评估改用 git 依赖并固定提交。
- 没有代码签名时，Windows SmartScreen 和 macOS Gatekeeper 会警告。对策：文档写明；签名需要用户的证书。

---

## 挂账：自己的 shell（`ut sh`）

**状态：挂账，未排期。** 用户 2026-11-17 决定：先不做，等以后做自己的 shell 时再说。下面是当时的调研和方案，原样保留。重新启动时先复核"已核实的事实"，那些会过期。

有两条路线，届时再选：
1. 嵌入 brush（下面的方案）。工作量约一个里程碑。
2. 从零写。工作量以年计（语法、执行、交互三块，Windows 没有 `fork`）。

### 目标

Windows、macOS、Linux 上用同一种 shell 语法（bash 兼容）。先作为可选项，不改默认。

### 已核实的事实（2026-11-17）

- brush：MIT；`brush-core` 0.5.0（要求 Rust 1.88，本项目 1.93）；依赖 tokio。
- 嵌入方式：`brush_core::Shell::builder().builtin(name, ...).build().await`，`shell.run_string(...)`。
- 交互层：`brush-interactive` 0.4.0，基于 reedline。
- 平台：Linux 和 macOS 是"日常可用"；**Windows 是"预览"，部分功能缺失**。
- 兼容性：2500 多个用例与 bash 对比。缺 `select`、`wait -n`、`disown`、部分 trap。
- 另有 `brush-coreutils-builtins` 0.1.0（把 uutils 做成内置命令）。

### 第 0 步：先验证的事实（验证性实现，不合并）

在分支 `spike/ut-sh` 上做。目的是决定"做"还是"不做"。

1. 给 `crates/ut` 加依赖 `brush-core`、`brush-builtins`、`brush-interactive`。加命令 `sh`：`ut sh -c "..."`、`ut sh FILE`、无参数时交互。
2. 写 60 条 shell 语言用例，存 `crates/ut/tests/shell/cases/*.sh`。覆盖：
   - 管道、`&&`、`||`、`;`、退出码（10 条）
   - 重定向 `>`、`>>`、`<`、`2>&1`、here-doc（8 条）
   - 变量、引号、`${x:-y}`、`${x#p}`、数组（10 条）
   - `$( )`、`( )`、算术 `$(( ))`（8 条）
   - `if`、`for`、`while`、`case`、函数、`local`（12 条）
   - 通配符、花括号展开、波浪号（6 条）
   - `trap EXIT`、后台 `&` 加 `wait`、`set -e`（6 条）
   用例只调用 `ut` 自带的命令，保证三个平台输入相同。
3. 在 WSL 里用真正的 bash 运行这 60 条，把 stdout、stderr、退出码存成 golden。
4. 用 `ut sh` 在 Windows 和 Linux 上运行，和 golden 逐字节比较。
5. 在 uni-term 里启动 `ut sh`（配置 `[shell] program = "<ut 的路径>"`, `args = ["sh"]`），用 `tools/shot.ps1` 实测 6 项：提示符出现；能输入并运行 `echo ok`；Ctrl-C 中断 `sleep 30` 并回到提示符；Tab 补全一个路径；上方向键取回上一条；改窗口大小后 `tput cols` 或 `$COLUMNS` 正确。
6. 测 `pwd`、`echo $PWD`、Tab 补全在 Windows 上输出的路径分隔符。
7. 测启动时间：`ut sh -c true` 连跑 20 次取中位数。测 release 版 `ut.exe` 的大小。
8. 查 `brush-coreutils-builtins` 固定的 uutils 版本，以及它怎样给内置命令做重定向。

### 通过线

| # | 条件 | 不满足时 |
|---|---|---|
| 1 | Windows 和 Linux 都能编译；`cargo clippy -D warnings` 通过 | 不做 |
| 2 | Linux：60/60 与 bash 一致 | 低于 57 条：不做 |
| 3 | Windows：至少 54/60（90%）。每条不一致都列出并判断是否可接受 | 低于 48 条（80%）：不做 |
| 4 | 交互 6 项全部通过 | 任何一项失败且两天内修不好：不做 |
| 5 | `ut sh -c true` 中位数不超过 100 ms | 记录，不阻塞 |
| 6 | release 版 `ut.exe` 不超过 45 MB（现在 22 MB） | 记录，不阻塞 |

"不做"时：把数字写进 `docs/verification-log.md`，删除分支，M3 只给系统 shell 写钩子。本节其余步骤跳过。

### 设计（通过后）

- **入口**：`ut sh`。不创建名为 `sh` 或 `bash` 的链接：别的工具调用 `bash` 时期望的是真正的 bash。
- **`ut` 进入命令目录**：已移到 M3 的步骤里做（`ut history`、`ut view`、`ut md` 也需要它）。
- **配置**：不加新配置项。`[shell] program = "ut"`, `args = ["sh"]`。uni-term 把名字 `ut` 解析为它旁边的 `ut`。
- **外部命令**：`cat`、`ls` 等先作为外部进程运行（shell 在 PATH 里找到命令目录）。不做进程内内置命令。只有第 0 步第 7 项测出进程启动是瓶颈时，才考虑内置化，并且先确认第 8 项的重定向做法可靠。
- **通配符**：shell 自己展开。启动时设置 `UT_NO_GLOB=1`，子进程 `ut` 不再展开。
- **路径**：`$PWD`、提示符、补全在 Windows 上显示 `C:/...`。不做 `/c/...` 形式，不做参数自动转换。
- **启动文件**：只读 `<配置目录>/uni-term/shrc`。不读 `~/.bashrc`（里面常有只适用于真 bash 的内容）。
- **终端标记**：`ut sh` 启动时如果 `TERM_PROGRAM=uni-term`，自动加载和 bash 相同的钩子脚本（M3 写）。
- **PowerShell 和 cmd 的托管保留**，仍是默认。

### 步骤

1. 把 `spike/ut-sh` 的代码整理进主分支：`crates/ut/src/sh.rs`（入口和 tokio 运行时）、依赖、`ut --help` 里的说明。
2. （已移到 M3。）
3. `shell.rs`：`program = "ut"` 解析为同目录的 `ut`。加单元测试。
4. `sh.rs`：设置 `UT_NO_GLOB`；Windows 上的路径显示；读 `shrc`。
5. 把 60 条用例接入 `cargo test`（`crates/ut/tests/shell.rs`），Windows 和 Linux 共用一份 golden。已知不一致的用例单独列表，写明原因，不得静默跳过。
6. `tools/verify.ps1` 加一节 `shell`：第 0 步第 5、6 项的 6+3 项检查。
7. `tools/linux-test.sh`：加 `ut sh` 与 bash 的对比。

### 验证

- `cargo test`：60 条用例，Windows 至少 54 条一致，Linux 60 条。
- 实机 9 项全部通过。
- M2 的 59 项实机检查不变（证明默认 shell 的行为没有变化）。
- 可复现性：60 条用例连跑 3 次，输出相同。

### 验收标准

1. 同一个脚本文件在 Windows 和 Linux 上用 `ut sh` 运行，输出逐字节一致（已知不一致的除外，且有清单）。
2. 在 uni-term 里把 shell 设为 `ut sh`，能正常交互。
3. `ut sh` 里 `pwd`、提示符、补全在 Windows 上都没有 `\`。
4. 不设置时，行为和 M2 完全相同。

### 风险

- brush 的 Windows 支持不够成熟。退路：通过线已经规定了"不做"的条件。
- brush 升级带来行为变化。对策：`Cargo.toml` 里固定小版本；60 条用例就是回归测试。
