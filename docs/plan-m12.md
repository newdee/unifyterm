# M12：提示符由主题配置（光标左边那一行）

2026-11-26。用户："光标左侧那一行的配置，可以借鉴 p10k，我们也允许在主题中配置"；看过 `demo/prompt.html` 后加了"最新的一行提示符有一道从左到右的亮光划过""配色支持渐变""光效只划过文字，空白不划""ut setting 和配置文件里新增这一块""光效几秒一次也支持配置"；并要求"先写好数据结构，再实现功能"。

## 和以前的决定不同的地方

M10 写过"提示符是 shell 画的，终端不画"（`docs/plan-m10.md`）。这次改成：打开 `prompt.enabled` 后，UnifyTerm 生成提示符的内容；不打开时，四个钩子照旧只包住用户自己的提示符。

## 事实（2026-11-26 在本机量的）

| 事实 | 数 |
|---|---|
| 调一次 `ut --version` | 中位数 15.7 ms（含 PowerShell 计时开销） |
| 调一次 `git rev-parse` / `git status --porcelain=v2 --branch` | 49.9 ms / 54.3 ms |
| 本机 Cascadia Code、Cascadia Mono、Consolas 有没有 U+E0B0、U+E0B4（powerline）、U+F113（Nerd Font） | 都没有 |
| 现在的四个钩子 | 只包住用户的提示符，前后打 OSC 133;A、B |

## 香农六法

- 简化：提示符就是一行带颜色的字。先只做"一组段、四种样式、一行或两行"，transient 以后做。
- 相似问题：starship、oh-my-posh 都是"shell 每次调一个程序，程序打印提示符"。照它们：`ut prompt`。p10k 的样式名（lean、classic、rainbow）借来。
- 换说法："配置提示符"其实是两件事：字的内容和颜色（shell 打印，`ut prompt` 生成），和动画（只有终端能画）。光效和分隔符归终端，渐变归 `ut prompt`。
- 推广：光效按 OSC 133 标记找提示符，所以对任何提示符都有效，用户自己的 p10k 也一样。分隔符由终端画，所以任何字体都能显示。
- 结构分析：数据结构 → 设置和主题的读法 → `ut prompt` → 钩子 → 终端画分隔符 → 光效。每一步能单独测。
- 逆向：要"换主题，提示符跟着换"，提示符的颜色就只能引用主题的颜色（ANSI 色名），不能写死。要"关着时不花时间"，钩子就要在开 tab 时决定调不调 `ut prompt`，不能每次问。

## 数据结构（先写这些，再写功能）

### 设置和主题文件里的 `[prompt]`（`crates/config`）

设置文件和主题文件用同一个表。每个字段都可以不写，三层合起来：设置文件的 > 主题的 > 内置默认。`enabled` 只能写在设置文件里：主题不能替用户打开它。

```rust
/// [prompt]: what the settings file or a theme gives. Left out: the layer under it says.
pub struct PromptConfig {
    pub enabled: Option<bool>,            // 只在设置文件里
    pub style: Option<PromptStyle>,       // lean | classic | rainbow | plain
    pub separator: Option<Separator>,     // sharp | round | slant
    pub lines: Option<u8>,                // 1 或 2
    pub left: Option<Vec<Segment>>,
    pub right: Option<Vec<Segment>>,
    pub gradient: Option<Gradient>,       // off | segments | whole
    pub colors: Option<Vec<Paint>>,       // whole 用的一组色，2 到 8 个
    pub dir_color: Option<Vec<Paint>>,    // 每段 1 个色，或 2 个（这一段自己的渐变）
    pub git_color: Option<Vec<Paint>>,
    pub git_dirty_color: Option<Vec<Paint>>,
    pub status_color: Option<Vec<Paint>>,
    pub duration_color: Option<Vec<Paint>>,
    pub venv_color: Option<Vec<Paint>>,
    pub time_color: Option<Vec<Paint>>,
    pub shine: Option<Shine>,             // off | once | loop
    pub shine_every: Option<f32>,         // 秒，1 到 600；loop 时用
}

pub enum PromptStyle { Lean, Classic, Rainbow, Plain }
pub enum Separator { Sharp, Round, Slant }
pub enum Gradient { Off, Segments, Whole }
pub enum Shine { Off, Once, Loop }
/// 一段显示什么。
pub enum Segment { Dir, Git, Status, Duration, Venv, Time }
/// 一个颜色：主题的 16 色之一（按名字），或 "#rrggbb"。
pub enum Paint { Ansi(Ansi), Rgb([u8; 3]) }
/// black red green yellow blue magenta cyan white，bright-black 到 bright-white。
pub enum Ansi { Black, Red, …, BrightWhite }
```

文件里的写法：

```toml
[prompt]
style = "rainbow"
separator = "round"
lines = 2
left = ["dir", "git"]
right = ["status", "duration", "venv"]
gradient = "segments"
colors = ["cyan", "blue", "magenta"]
dir_color = ["blue", "magenta"]
shine = "loop"
shine_every = 4
```

不变量（读文件时检查，错了说出键名，和别的设置一样）：

- `lines` 是 1 或 2；`shine_every` 在 1 到 600 之间。
- `colors` 有 2 到 8 个；每段的颜色有 1 或 2 个。
- `left` 和 `right` 里同一段不出现两次，两边合起来也不重复。
- 颜色名只能是那 16 个名字，或者 `#rrggbb`。
- 主题文件写 `enabled` 时拒绝，说明它属于设置文件。

### 合起来之后（没有 `Option`）

```rust
/// The prompt in use: the three layers put together. Nothing left to choose.
pub struct Prompt {
    pub enabled: bool,
    pub style: PromptStyle,
    pub separator: Separator,
    pub lines: Lines,                     // One | Two
    pub left: Vec<Segment>,
    pub right: Vec<Segment>,
    pub gradient: Gradient,
    pub colors: Vec<Paint>,
    pub paints: SegmentPaints,            // 每段的 1 或 2 个色，git 有两套
    pub shine: Shine,
    pub shine_every: Duration,
}
```

内置默认：`enabled = false`，lean，sharp，两行，左 `dir git`，右 `status duration venv`，不渐变，`colors = cyan blue magenta`；dir 蓝，git 绿（有改动时黄），status 红，duration 黄，venv 青，time bright-black；不扫光，`shine_every = 4`。

### `ut prompt` 读到的事实

```rust
/// What the prompt is about, gathered once a prompt.
pub struct Facts {
    pub shell: ShellKind,                 // pwsh | powershell | bash | zsh | fish
    pub cwd: PathBuf,
    pub home: Option<PathBuf>,
    pub status: Option<i32>,              // 上一条命令的退出码；没有命令：None
    pub took: Option<Duration>,           // 上一条命令的耗时
    pub git: Option<Repo>,
    pub venv: Option<String>,             // VIRTUAL_ENV 或 CONDA_DEFAULT_ENV 的最后一段
    pub width: Option<u16>,               // 终端宽度，两行时把右边的段放到第一行末尾
    pub now: (u8, u8, u8),
}

pub struct Repo {
    pub head: Head,                       // Branch(String) | Detached(String：短提交号)
    pub changes: Option<Changes>,         // 时间上限内没算完：None，只显示分支
}

pub struct Changes { pub staged: u32, pub changed: u32, pub untracked: u32, pub ahead: u32, pub behind: u32 }
```

分支直接读 `.git/HEAD`（向上找 `.git`，`.git` 是文件时按 `gitdir:` 找）。改动数跑 `git status --porcelain=v2 --branch`，超过 300 ms 就不等，只显示分支。

### `ut prompt` 生成的结果

```rust
/// One cell's worth of the prompt: a character and its colors, before any shell's escaping.
pub struct Cell { pub text: char, pub fg: Option<[u8; 3]>, pub bg: Option<[u8; 3]>, pub bold: bool }
/// The prompt, line by line, and where each line's right part begins (two lines, or zsh and fish's right prompt).
pub struct Rendered { pub lines: Vec<Line> }
pub struct Line { pub left: Vec<Cell>, pub right: Vec<Cell> }
```

颜色在这里已经是 24 位的数：主题的颜色、渐变都在生成 `Cell` 时算好，每格一个色。最后一步才按 shell 转义：PowerShell 和 fish 原样；bash 用 `\001 \002` 包住转义序列，`\`、`$`、反引号要转义；zsh 用 `%{ %}`，`%` 写成 `%%`。

分隔符用 powerline 的码位，由终端画：

| 分隔符 | 左边（指向右） | 右边（指向左） | 细线 |
|---|---|---|---|
| sharp | U+E0B0 | U+E0B2 | U+E0B1、U+E0B3 |
| round | U+E0B4 | U+E0B6 | U+E0B5、U+E0B7 |
| slant | U+E0BC | U+E0BA | U+E0BD、U+E0BB |

plain 样式只用 ASCII，不用这些码位。

### 终端一侧（`crates/unifyterm`）

```rust
/// Where the newest prompt is on the screen: from its 133;A to its 133;B (and the row of the cursor after).
pub(crate) struct PromptSpan { pub start: Place, pub input: Place }

/// The light that crosses the newest prompt.
pub(crate) struct Sweep {
    pub began: Instant,
    pub every: Option<Duration>,          // Loop：每隔多久；Once：None
}

/// How bright a lit cell is at a moment: 0 (as drawn) to 1 (brightest). Blank cells are never lit.
fn light(sweep: &Sweep, now: Instant, way: f32, length: f32) -> f32
```

`way` 是这一格在光的路线上的位置：只算有字的格，超过 2 格的空白压缩成 2 格。

画分隔符：新模块 `powerline.rs`，和 `boxdraw.rs` 一样按码位给出形状（三角、半圆、斜角、细线），终端自己画，不取字体。

## 步骤

1. `crates/config`：上面的类型、读法、三层合并、检查、`ut setting` 的项目表（`KEYS` 加 `prompt.*`）、单元测试。主题文件的 `[prompt]` 不再算"不认识的表"。
2. 内置主题各配一个 `[prompt]`（样式和颜色，不开光效）。
3. `ut prompt`：收集事实、生成 `Rendered`、按 shell 转义；`ut prompt preview` 打印示例。单元测试覆盖每种样式、渐变、对齐、转义。
4. 钩子：开 tab 时 `prompt.enabled` 为真，就给 shell 设 `UNIFYTERM_PROMPT=1`；四个钩子见到它就每次调 `ut prompt`。`prompt.enabled` 的改动由下一个 tab 使用，其余 `prompt.*` 在下一个提示符就生效。
5. 终端画 powerline 分隔符。
6. 光效：按最新提示符的位置，按格调亮，空白格不动；`once` 在新提示符出现时扫一次，`loop` 每 `shine_every` 秒一次；系统设了减少动画时不扫。
7. README、`docs-check`、网页；三轮验收；发版 0.7.0。

## 实现时改动的决定

| 计划里写的 | 实际 | 原因 |
|---|---|---|
| 颜色由 `ut prompt` 自己从主题算 | 新数据结构 `Palette` 放在 `crates/config`，窗口算 bar 色的比例 `BAR_AMOUNT` 和两色相混 `toward` 也搬过去，窗口和 `ut` 读同一处 | 主题没给 bar 时由前景和背景算出；两处各算一遍迟早不一样 |
| 每段的颜色写成 `dir.color` | 写成 `prompt.dir_color` 等 7 个平的键 | `ut setting` 的键是"表.键"两层；三层的键它不认 |
| `Segments`、`Paints` 当作普通文字 | `ut setting` 多两种值的类型，逐个检查名字，并给出可选列表 | 写错的段名、颜色名在设置界面里就被拒绝 |
| git 状态的预算 ≤ 80 ms | 本机这个仓库里约 90 ms（不读 git 时 18 ms，`git status` 本身约 57 ms） | 超了约 10 ms；仍在 300 ms 的上限内。以后可以把 git 状态按 `.git/index` 的修改时间缓存 |
| 光效在提示符出现后一直可以扫 | 只在 shell 停在提示符时扫（`Session::at_prompt`：有输入标记，没有命令在跑，不是全屏程序） | 第一轮 review：命令运行时，全屏程序的字、或历史满了之后标记的行号移动，会让光扫到别的字上 |
| 钩子只改 PowerShell | fish 原有的 `preexec` 一行一起修了 | 同一个写法（fish 单引号里 `\\` 是一个反斜杠，放在格式中间会吃掉后一个字符）从 fish 钩子写成起就在屏幕上印出 `e]133;C\`，新的提示符行也印出 `%s` |
| 在真实窗口里看效果 | 加了 `verify.ps1` 的 `prompt` 一节（4 项） | 以后每轮都实机检查提示符开、关和光效 |

## 这一版不做

- transient（旧提示符收成一个 ❯）：PowerShell、zsh、fish 的做法各不相同，bash 做不到。等这一版用过再做。
- cmd.exe：它的 `PROMPT` 是固定的字，没有退出码和 git。照旧只包住。
- Nerd Font 图标：默认字体没有。
