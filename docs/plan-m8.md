# 实施计划：M8 用一句话写命令；更多的模型来源

状态：2026-11-23 的设计稿，2026-11-24 已按它实现。实现时改掉的决定列在文末"实现时改动的决定"一节：正文与那一节不一致时，以那一节为准。规则沿用 `docs/plan.md` 第 0 节（固定流程、代码规则、测试规则）。

## 目标

1. 在提示符下输入 `# 一句话` 再按 Enter，UnifyTerm 把这一行换成一条命令。命令由模型写，默认不执行。
2. 模型来源从 2 种增加到 3 类，用户按自己有的东西选一种：
   - 云端的接口：Anthropic、OpenAI，以及所有用 OpenAI 接口形状的服务；
   - 本机部署的模型服务：Ollama、LM Studio、llama.cpp、vLLM；
   - 本机已经装好并登录的 agent 命令行：Claude Code、Codex、Cursor CLI、pi，或任意一个命令。
3. 不带模型，不下载模型。小模型是用户自己的选择，UnifyTerm 只提供接入方式。

不做的事：把 llama.cpp 编进 UnifyTerm；替用户下载或启动模型服务；自己训练模型。

## 已经定下来的事

| 决定 | 谁定的 | 原因 |
|---|---|---|
| 入口是行首的 `#`，不是快捷键，也不弹输入框 | 用户，2026-11-23 | Cmd-Shift 再加一个键太难按 |
| 小模型可选，只提供接入选项 | 用户，2026-11-23 | 约 1 GB 的下载和 1.3 GB 的内存不该是默认 |
| 同时支持本机部署的接口、OpenAI 接口、agent 命令行 | 用户，2026-11-23 | |
| 默认手动回车，另有自动回车模式 | 用户提出，2026-11-23 | |
| 自动回车用白名单，不用黑名单 | 本文 | 见"设计 4" |

## 分析（香农六法）

- **简化**：去掉所有来源的差别，问题只剩一句话："给模型一段文字，拿回一段 JSON"。现有的 `Provider::complete` 就是这句话。所以新来源只是它的新实现，功能代码不用知道来源。
- **相似问题**：`fix` 已经是"一条错的命令 → 一条对的命令"。`ask` 是"一句话 → 一条命令"：请求、脱敏、校验、填进输入行都相同。Warp 用行首 `#` 输入自然语言；lxask 用 Ctrl+G 把这一行换成命令，从不执行。
- **换说法**：不问"怎样支持 Ollama、LM Studio、llama.cpp、vLLM、OpenAI"，问"它们的差别在哪"。差别只有三处：地址、要不要密钥、认不认 `response_format` 里的 `json_schema`。所以是一种实现加一张表，不是五种实现。
- **推广**：agent 命令行推广成"任意一个读入文字、打印文字的命令"。Claude Code、Codex 只是这张表里预先填好的几行。
- **结构分析**：拆成 4 组，互不依赖，可以分别做完、分别验收：A `ask` 功能；B 接口的预设；C 命令行来源；D 评测和文档。
- **逆向**：最坏的结果有两个。一是自动回车执行了删东西的命令：所以自动回车只对"只读"的命令生效。二是 agent 命令行自己去运行了命令或改了文件：所以每个预设都必须关掉它的工具，并且在空目录里运行。

## 第 0 步：先验证的事实

已经验证（2026-11-23，本机）：

1. `claude` 2.1.330 有 `-p`、`--output-format`、`--tools ""`（不给任何工具）、`--system-prompt`、`--model`、`--no-session-persistence`、`--json-schema`。
2. `codex` 0.142.5 的 `codex exec`：没有提示词参数时从标准输入读；有 `--sandbox read-only`、`--skip-git-repo-check`、`--ephemeral`、`--output-schema FILE`、`-o FILE`、`--color`、`-m`。
3. `pi` 0.73.1 有 `-p`、`--no-tools`、`--no-session`、`--system-prompt`、`--model`、`--mode json`。
4. `Session::input()` 只取提示符到光标之间的字。光标右边的灰色预测（PSReadLine、zsh-autosuggestions）不在里面。

还要验证（做每一组之前先做对应的几条，结果写进验收记录）：

| # | 要验证的事 | 属于 | 怎么验证 |
|---|---|---|---|
| 1 | Ollama、LM Studio、llama.cpp `llama-server`、vLLM 的默认地址，和 `/v1/chat/completions` 认不认 `response_format: json_schema` | B | 读各自的文档；有服务的机器上各发一次请求 |
| 2 | OpenAI 的接口：新模型要 `max_completion_tokens` 还是 `max_tokens`；`json_schema` 的 `strict` 对 schema 的要求 | B | 读 OpenAI 的文档 |
| 3 | 小模型把思考过程写在 `<think>…</think>` 里时，回答在哪个字段 | B | 用一个带思考的本地模型发一次 |
| 4 | `claude -p` 没有提示词参数时从标准输入读；`--tools ""` 时它不能运行命令；用户的 hook 会不会改它的输出 | C | 让它"建一个文件"，检查文件不存在；对比有 hook 和没有 hook 的输出 |
| 5 | `codex exec` 的最后一条回答打印在标准输出，过程打印在标准错误 | C | 运行一次，两个流分开存 |
| 6 | `pi -p` 能不能从标准输入读提示词 | C | 运行一次 |
| 7 | Cursor CLI 的程序名和参数（本机没有装） | C | 读它的文档；要用户在装了的机器上运行一次 |
| 8 | 三个命令行各问 5 次同一句话的耗时（冷启动和连续问） | C | 记中位数和最大值；决定超时 |
| 9 | Windows 上启动 npm 装的 `.cmd` 命令行：不闪黑窗口；超时后整棵进程树都结束 | C | 超时后数剩下的进程，应为 0 |
| 10 | macOS 从程序坞启动时找得到这些命令行（它们在登录 shell 的 `PATH` 里） | C | 用现有的"登录 shell 的环境"取 `PATH`，在那台 Mac 上试 |
| 11 | 各 shell 里，以 `#` 开头的输入行，`Session::input()` 读到的字和打的字一样（含中文、含全角空格） | A | pwsh、bash、zsh、fish 各打 5 行，逐行比较 |

第 7 条没有验证之前，`cursor` 这个预设不写进代码。

## 设计

### 1. 三类来源，一个接口

`crates/assist` 里的 `Provider` 不变：

```rust
pub trait Provider: Send + Sync {
    fn complete(&self, request: &Request) -> Result<String, Error>;
}
```

| 实现 | 状态 | 说什么协议 |
|---|---|---|
| `Anthropic` | 已有，不改 | Messages API |
| `OpenAiCompatible` | 已有，要改（设计 2） | `POST {base}/chat/completions` |
| `Command` | 新增（设计 3） | 启动一个程序：提示词写进它的标准输入，读它的标准输出 |

`suggest`、`fix`、`explain` 和新的 `ask` 都只认识 `Provider`，不知道来源是哪一类。

### 2. OpenAI 形状的接口：一种实现，一张表

**配置里的名字**（`ai.provider`）。每个名字只是一组默认值；`ai.base_url` 写了就用写的。

| `ai.provider` | 默认地址 | 密钥 | 说明 |
|---|---|---|---|
| `openai` | `https://api.openai.com/v1` | 必须 | 新增 |
| `ollama` | `http://localhost:11434/v1` | 不要 | 新增 |
| `lm-studio` | `http://localhost:1234/v1` | 不要 | 新增 |
| `llama-cpp` | `http://localhost:8080/v1` | 不要 | 新增 |
| `vllm` | `http://localhost:8000/v1` | 不要 | 新增 |
| `openai-compatible` | 无，必须写 `ai.base_url` | 可选 | 已有。其余所有服务都用它：LocalAI、Jan、KoboldCpp、OpenRouter、DeepSeek…… |
| `anthropic` | Anthropic 的地址 | 必须 | 已有 |

表里的地址是第 0 步第 1 条要核对的。这张表放在 `crates/config/src/providers.rs`，窗口和 `ut setting` 共用，`ut setting` 的选择列表从它生成。

**各服务的差别，由程序自己试出来**，不让用户配：

1. **回答的格式**。第一次请求带 `response_format: {type: "json_schema", …}`：它约束模型只能生成符合 schema 的字，小模型最需要它。服务答 HTTP 400 时，改用 `json_object` 再发；还是 400，就不带 `response_format` 再发。试出来的结果记在这个 `OpenAiCompatible` 里（进程存活期间），之后的请求直接用它。
2. **长度参数**。`openai` 用 `max_completion_tokens`，其余用 `max_tokens`（待第 0 步第 2 条确认）。
3. **思考过程**。解析之前先去掉 `<think>…</think>`。现在的 `parse` 取"第一个 `{` 到最后一个 `}`"，思考过程里的花括号会让它取错。
4. **密钥**。`ai.api_key_env` 为空就不发 `authorization`（已有的行为）。

**超时**。本机的模型第一次回答要先加载，可能要十几秒。`suggest` 的 3 秒不变：超时就安静地没有建议。`fix` 15 秒、`explain` 20 秒不变。`ask` 30 秒。命令行来源的超时另定：先按 60 秒，等第 0 步第 8 条的数字出来再改。

### 3. agent 命令行：`Command`

**配置**

| `ai.provider` | 启动的命令（`{…}` 由程序填） |
|---|---|
| `claude-code` | `claude -p --output-format text --tools "" --no-session-persistence --system-prompt {system}` |
| `codex` | `codex exec --sandbox read-only --skip-git-repo-check --ephemeral --color never` |
| `pi` | `pi -p --no-tools --no-session --system-prompt {system}` |
| `cursor` | 待第 0 步第 7 条 |
| `command` | 用户在 `ai.command` 里写的命令，例如 `["my-agent", "--quiet"]` |

- `ai.model` 不为空时，预设加上各自的选模型参数（`--model`、`-m`）。为空就用命令行自己的默认模型。
- 命令里有 `{prompt}`：提示词放在那个参数里。没有：提示词写进标准输入。预设都用标准输入：不受命令行长度限制，引号不会出错，别的进程也看不到。
- 命令里没有 `{system}`：指示和问题合成一段文字发过去。
- 回答的 JSON schema 写在提示词的末尾（和 `OpenAiCompatible` 现在的做法相同）。标准输出的全部文字交给现有的 `parse`。

**运行的方式**

1. 工作目录是数据目录下的一个空目录 `agent-cwd`。命令行不会读到用户项目里的 `CLAUDE.md`、`AGENTS.md`，也没有项目文件可以读。
2. 预设必须带"关掉工具"和"不留会话记录"的参数。有一个单元测试逐个预设断言这两点。
3. 超时：到时间就结束整棵进程树。Windows 上用作业对象，Unix 上用进程组。
4. Windows 上不开控制台窗口（复用 `shell::no_window`）。
5. 找程序：先在窗口进程的 `PATH` 里找，再在登录 shell 的 `PATH` 里找（macOS 从程序坞启动时需要）。找不到时，设置页和弹窗里说明"`claude` 没有找到"。
6. 退出码不是 0：`Error::Status`，带标准错误的前 300 个字。

**限制，写进文档**

- 命令行来源**不提供灰色建议**（`ai.suggest` 对它们不生效）。每问一次要启动一个进程，耗时是秒级，而且每次都计入用户的订阅额度。打字时每停 300 毫秒问一次是不可接受的。
- 用户按 Esc 关掉等待的弹窗后，那个进程继续运行到超时，回答被丢掉。先不做取消。

**同意**。和现在相同：第一次使用前弹窗问一次。同意的对象是"哪个来源、哪个模型、哪个地址或哪条命令"；换了其中任何一个就重新问。

### 4. `ask`：一句话 → 一条命令

**入口**。按 Enter 时，下面 5 条都成立才拦下这一次 Enter：

1. shell 在输入行（`Session::input()` 有值：不在全屏程序里，没有往回滚，输入只有一行）；
2. 输入以 `#` 开头，去掉 `#` 和空白后还有字；
3. `ai.ask` 是 `true`，并且配置了来源；
4. UnifyTerm 知道这是哪个 shell（`fill` 需要知道怎样清空输入行）；
5. 用户没有在本次运行里拒绝过发送。

任何一条不成立：Enter 照常交给 shell。shell 把这一行当作注释，什么都不执行。**没有配置来源时，行为和现在逐字节相同。**

**请求**（`crates/assist` 新增）

```rust
pub struct Ask<'a> {
    pub words: &'a str,          // "#" 后面的字
    pub cwd: &'a str,
    pub shell: &'a str,
    pub system: &'a str,         // "Windows" | "macOS" | "Linux"
    pub recent: &'a [String],    // 最近 5 条命令，新的在前
}
pub struct Asked { pub command: String, pub note: String }
pub fn ask(provider: &dyn Provider, ask: &Ask<'_>) -> Result<Option<Asked>, Error>;
```

- 发出去的每一段文字都先过 `clean`（密钥替换掉，主目录写成 `~`）。
- 回答是 `{"command": "...", "note": "一句话说明"}`。`command` 为空表示模型不知道：返回 `None`。
- `command` 里有换行：丢掉，`Error::BadAnswer`。换行会让前半条命令直接执行。
- 每一部分的解释不在这一次请求里要。命令填进输入行以后，Ctrl-Shift-E 照常解释它。原因：一次请求两个约束，小模型更容易整条作废。

**回答到了以后**

1. 输入行还是当初问的那一行：用现有的 `fill` 把它换成命令。用户已经改了这一行：不动它，只显示弹窗。
2. 弹窗 `Popup::Asked` 显示 `note`。命令里的程序在本机找不到时，多一行"`xxx` 在这台机器上没有找到"（用 `Helper` 已有的程序名列表）。
3. 这个弹窗不占用按键：按 Enter 就运行输入行里的命令，按别的键就继续编辑，弹窗随之关掉。
4. 没有回答（超时、被拒绝、格式不对）：输入行不动，弹窗显示一行原因。

**回车的方式**：`ai.ask_enter`

| 值 | 行为 |
|---|---|
| `"manual"`（默认） | 只填进输入行。用户看过以后自己按 Enter |
| `"auto"` | 命令是"只读"的：填进去并立刻回车。其余：和 `manual` 相同，弹窗多一行"这条命令会改东西：没有自动运行" |

**什么算"只读"**：`crates/assist` 里的纯函数 `fn runs_alone(command: &str, shell: &str) -> bool`。下面 3 条都成立才是 `true`：

1. 没有重定向（`>`、`>>`）、没有命令替换（`$(…)`、反引号）、没有 `sudo`、没有 `&` 后台运行；
2. 按 `|`、`&&`、`;` 拆开后，每一段的程序名都在"只读"表里。初始的表：`ls`、`cat`、`head`、`tail`、`grep`、`wc`、`du`、`df`、`ps`、`pwd`、`which`、`sort`、`diff`、`file`、`stat`、`find`、`echo`，PowerShell 的 `Get-*`、`Select-*`、`Measure-*`、`Where-Object`、`Sort-Object`、`Format-Table`、`Format-List`、`Test-Path`，以及 `git` 的 `status`、`log`、`diff`、`show`、`branch`（不带参数时）；
3. 表里的程序带了会写东西的选项时不算：`find` 的 `-delete`、`-exec`、`-ok`，`sort` 的 `-o`。

为什么用白名单：黑名单总有漏的（`shred`、`truncate`、`find -delete`、`xargs rm`、`curl … | sh`）。白名单认不出的命令只是多按一次 Enter，这个代价可以接受。

表里每加一个程序，先查它有没有会写东西的选项或参数。例子：`uniq` 的第 2 个文件参数是输出文件，`date -s` 改系统时间，`Format-Volume` 格式化磁盘：所以表里没有 `uniq`、`date`，PowerShell 的 `Format-` 只列出具体的两个。这张表只有一处，用表格式的测试覆盖。

### 5. 配置

```toml
[ai]
provider = "none"      # none | anthropic | openai | openai-compatible | ollama | lm-studio | llama-cpp | vllm
                       #      | claude-code | codex | pi | command
model = ""
base_url = ""          # 预设有默认地址；openai-compatible 必须写
api_key_env = ""
command = []           # 只在 provider = "command" 时用
suggest = true         # 命令行来源不提供
fix = true
explain = true
ask = true             # 行首 # 的一句话
ask_enter = "manual"   # manual | auto
```

校验（`Config::parse` 里，不合格的值不写入，并说明原因）：

- `openai-compatible`：`base_url` 必须有（已有）。
- `anthropic`、`openai`：`api_key_env` 必须有。
- `command`：`ai.command` 至少有一项。别的来源写了 `ai.command`：报错，说明它只对 `command` 生效。
- 接口类来源：`model` 必须有（已有）。命令行类来源：`model` 可以为空。

设置项从 32 个变成 35 个（`ai.command`、`ai.ask`、`ai.ask_enter`）。要同步改的地方：`crates/config/src/keys.rs`、`README.md` 的设置表、`site/` 里的数字、`tools/docs-check.ps1` 检查的数量。

弹窗里"会发送什么"的那句话加上"`#` 后面你写的那句话"。

### 6. 影响到的现有行为

| 现有行为 | 会不会变 | 靠什么保证 |
|---|---|---|
| 没有配置来源时的全部行为 | 不变 | 入口第 3 条；测试：假服务收到 0 个请求，`# x` 回车后 shell 收到的字节和现在相同 |
| `openai-compatible` 现有的用户 | 第一次请求多带 `json_schema`；服务不认时多 1 到 2 次请求，之后不再试 | 假服务测试：只认 `json_object` 的服务，第 2 次起每个问题 1 个请求 |
| `anthropic` | 不变 | 代码不改 |
| 在提示符下写注释的用户 | 配了来源以后，注释会被拿去问模型 | `ai.ask = false` 关掉；README 写明 |
| `fix` 弹窗里 Enter 不运行命令 | 不变 | `Popup::Asked` 是新的一种，别的弹窗的按键不改 |

## 步骤

每一组做完都走一次三轮验收，数字写进 `docs/verification-log.md`。四组可以分开发布。

**A. `ask`（用现有的两种来源就能用）**

1. 第 0 步第 11 条。
2. `crates/assist`：`ask`、`runs_alone` 和它们的单元测试。
3. `crates/config`：`ai.ask`、`ai.ask_enter`，校验，`keys.rs`。
4. `helper.rs`：`Feature::Ask`、`Helper::ask`；"会发送什么"的文字。
5. `terminal.rs`：Enter 的入口判断、`Pending::Ask`、`Popup::Asked`、自动回车。
6. `tools/verify.ps1` 的 `ai` 一节加 `ask` 的检查（假服务）。

**B. 接口的预设**

1. 第 0 步第 1、2、3 条。
2. `crates/config/src/providers.rs`：那张表；`Provider` 枚举加 5 个名字；校验。
3. `OpenAiCompatible`：格式的三级回退并记住结果；`max_completion_tokens`；去掉 `<think>`。
4. `ut setting`：来源的选择列表从表生成；选了预设后 `base_url` 一行显示默认地址。
5. 假服务测试：三种服务（认 `json_schema`、只认 `json_object`、都不认）。

**C. agent 命令行**

1. 第 0 步第 4 到 10 条。
2. `crates/assist/src/command.rs`：`Command`，占位符，标准输入，超时结束进程树，错误。
3. 预设的表（和 B 的表在同一个文件）；`ai.command`；校验；`suggest` 对命令行来源不生效。
4. 找程序（登录 shell 的 `PATH`）；找不到时的提示。
5. 测试：用系统自带的命令当假命令行（Windows `cmd /c type`，Unix `cat`、`sleep`）：正常回答、超时、退出码非 0、程序不存在。

**D. 评测和文档**

1. 评测集：30 句请求（中文 15 句、英文 15 句），pwsh 和 zsh 各一份期望。期望由人写，不由模型写。每个可用的来源跑一遍，记下：命令可用的比例、中位耗时、被 `BadAnswer` 丢掉的次数。
2. README：`# 一句话` 一段；设置表；每个来源一个最短的配置例子。
3. 网页、skill（`skills/unifyterm/SKILL.md`）、设置数量。对外文案由用户定稿：我只写事实，写完告诉用户改了哪里。

## 验证

**单元测试**

- `ask`：正常回答；空命令；带换行的命令；格式错误的 JSON；超时；发出去的文字里没有密钥（复用现有的密钥测试集）。
- `runs_alone`：40 条命令的表，20 条只读（期望 `true`），20 条会改东西（期望 `false`），pwsh 和 POSIX 的写法都有。
- `OpenAiCompatible`：三种假服务各自的请求次数；带 `<think>` 的回答解析正确。
- `Command`：占位符填对；没有 `{prompt}` 时提示词在标准输入里；每个预设都带关工具和不留会话的参数；超时后进程数为 0。
- 配置：每个来源的名字都能解析；每条校验规则各有一个不合格的例子。

**实机检查**（假服务和假命令行都记录收到的内容）

1. `provider = "none"`：输入 `# 列出文件` 回车，假服务收到 0 个请求，shell 收到这一行。
2. 配了假服务：输入 `# 列出文件` 回车，输入行变成假服务给的命令，命令没有运行。再按 Enter，命令运行。
3. 假服务的回答带换行：输入行不变，弹窗显示原因。
4. `ask_enter = "auto"`：回答 `ls -la` 时自动运行；回答 `rm -rf x` 时只填进去，弹窗有"没有自动运行"。
5. 请求还没回来时改了输入行：回来后输入行是用户改的那一行。
6. 第一次使用：先出现同意弹窗；拒绝后收到 0 个请求。
7. 在 vim 里（全屏程序）输入 `# x` 回车：vim 收到这些键，没有请求。
8. 假命令行一直不回答：终端仍然可以打字；超时后没有剩下的进程。

**真实的来源**（各手工走一次 `ask`、`fix`、`explain`，结果写进验收记录）

- 我能做的：本机的 Claude Code、Codex、pi。每个来源问不超过 10 次。
- 需要用户的：OpenAI 的密钥；一台装了 Ollama 或 LM Studio 的机器；Cursor CLI。

## 验收标准

1. 没有配置来源时，行为和 0.2.4 相同：假服务 0 个请求。
2. 用户同意之前，没有任何文字发出去，也没有任何命令行被启动。
3. 带换行的回答从不进入输入行。
4. `auto` 模式下，白名单以外的命令从不被自动回车：40 条的表全部符合期望。
5. 命令行来源在超时后不留下进程。
6. 评测集的数字写进验收记录。没有数字的来源，文档里不写它"可用"。

## 风险

| 风险 | 对策 |
|---|---|
| 小模型给出不存在的参数 | 弹窗只说明"程序找不到"；参数对不对由用户看。后续的命令清单协议（路线图里的下一项）可以把真实的参数交给模型 |
| agent 命令行换了版本，参数变了 | 预设集中在一张表里；退出码非 0 时把它的报错显示出来；用户可以改用 `command` 自己写 |
| 命令行来源的耗时让人以为卡住了 | 等待时弹窗显示"正在问 claude…" |
| 用户的 hook 或全局指示改了命令行的输出 | 在空目录里运行；`--system-prompt` 换掉默认指示；`parse` 只取 JSON 的部分 |
| `#` 开头的注释被拿去问模型 | 只在配了来源并且 `ai.ask` 为 `true` 时；README 写明怎样关 |

## 以后再说（不在本期）

- 不写 `#`，直接打一句中文：shell 报"找不到命令"后，由 `fix` 的弹窗接手。英文句子不行（`find big files` 会被当成 `find` 运行），所以只能是补充。
- UnifyTerm 自己下载并管理 `llama-server`。等评测集的数字出来再决定。
- 关掉弹窗时取消还在运行的请求。

## 实现时改动的决定（2026-11-24）

第 0 步验证出来的事实，和写代码时发现的问题，让下面这些地方和正文不同。

| 正文里写的 | 实际做的 | 原因 |
|---|---|---|
| 预设 `llama-cpp`，默认地址 `http://localhost:8080/v1` | 没有这个预设。llama.cpp 用 `openai-compatible`，自己写 `ai.base_url` | llama.cpp 的默认端口随版本变：它现在的 README 写 9931，旧版本是 8080。一个对一半用户是错的默认值，不如没有 |
| 预设 `cursor`，待验证 | 没有这个预设 | Cursor CLI 的文档写明：print 模式不带 `--force` 也会写工作区里的文件。它没有"只回答"的开法，不满足"必须关掉工具"这一条。想用的人用 `command` |
| `pi` 要实际运行一次 | 参数和"print 模式读标准输入"按它自带的文档核对；没有实际问过 | 这台机器上的 pi 没有登录（退出码 1，"Use /login"） |
| 命令里有 `{system}` 占位符；`claude` 用 `--system-prompt {system}` | 没有 `{system}`。指示、回答的形状、问题合成一段文字，全部走标准输入。`claude` 的 `--system-prompt` 是一句固定的短话 | npm 装的命令行在 Windows 上是 `.cmd`：带换行或引号的参数传给批处理文件不安全，Rust 会直接拒绝。固定的短参数没有这个问题 |
| 别的来源写了 `ai.command` 就报错 | 不报错，那一项只是不用 | 在 `ut setting` 里换一下来源再换回来是常事；换走就报错，等于逼人先删掉它 |
| 命令行来源的超时先按 60 秒 | 90 秒 | 实测：Claude Code 一次 9 到 11 秒（5 次），Codex 一次 20 到 39 秒（3 次）。60 秒离 39 秒太近 |
| Windows 上用作业对象结束进程树 | Windows 用 `taskkill /T /F`，Unix 用 `kill -KILL -- -进程组` | 这些命令行问一次就自己结束，不需要"终端退出时带走它"。两条系统自带的命令不用 `unsafe`，不加依赖。有一个测试证明孙进程也被结束了 |
| 弹窗里说明"`xxx` 在这台机器上没有找到" | 没做 | 判断"找不到"要知道 shell 的内建命令、别名和函数；只看 `PATH` 会把 `cd`、`mkdir`（PowerShell 的函数）这类报成找不到。准确的来源是路线图里的命令清单协议，等它 |
| `runs_alone(command, shell)`，只读表里有 `file`、`stat`、`find`…… | 函数叫 `only_reads`。表里没有 `file`（`file -C` 会写文件）。带 `!` 的命令不算（bash 和 zsh 的历史展开会把以前的整条命令放进来）。带 `( ) { }` 的不算（PowerShell 的脚本块里可以是任何命令）。cmd.exe 里什么都不算 | cmd.exe 先把 `%变量%` 的值放进这一行，再解析这一行：变量里可以藏第二条命令 |
| 模型给的命令以 `#` 开头 | 当作"没有命令" | 它填进输入行以后，再按 Enter 又会被当成一句话拿去问 |
| 设置项 32 → 35 | 相同：`ai.command`、`ai.ask`、`ai.ask_enter` | |

第 0 步的结果（2026-11-23 到 24，本机）：

| # | 结果 |
|---|---|
| 1 | Ollama `http://localhost:11434/v1`，文档列出支持 `response_format`；LM Studio `http://localhost:1234/v1`，支持 `json_schema`，没有写 `json_object`；vLLM 的例子都是 `http://localhost:8000/v1`，默认支持 `json_schema`；llama.cpp 见上表。没有在真实的服务上发过请求（这台机器上一个都没装） |
| 2 | OpenAI 的文档：`max_tokens` 已废弃，o 系列模型不接受它，改用 `max_completion_tokens`。所以 `openai` 用后者，其余用 `max_tokens` |
| 3 | llama.cpp 默认把思考放在 `message.reasoning_content`，也可以留在 `content` 的 `<think>` 里；vLLM 放在 `message.reasoning`。所以只读 `content`，并且先去掉最后一个 `</think>` 之前的全部内容 |
| 4 | `claude -p` 没有提示词参数时读标准输入：是。`--tools ""` 时让它建一个文件：它回答 `{"done": false}`，文件不存在。用户自己的 hook 不改变标准输出里的 JSON |
| 5 | `codex exec`：最后的回答在标准输出，过程在标准错误。`--sandbox read-only` 时让它建文件：回答 `{"done": false}`，文件不存在 |
| 6、7 | 见上表（pi、Cursor） |
| 8 | 见上表（耗时） |
| 9 | 超时后孙进程也结束：单元测试 `a_program_that_takes_too_long_is_stopped_with_what_it_started`；去掉结束进程树的那一行，这个测试失败 |
| 10 | 找程序：先在窗口进程的 `PATH` 里找，再问登录 shell（`$SHELL -l -c 'printenv PATH'`），并用那个 `PATH` 运行它。没有在从程序坞启动的 Mac 上试过 |
| 11 | 实机检查：pwsh 里英文和中文的句子，模型收到的字和打的字相同 |
