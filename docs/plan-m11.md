# M11：按键可配置、paper 两套主题、macOS 上的两处

2026-11-25。用户用过 0.5.0 后提的四件事。

## 用户的话和对应的做法

| 用户说的 | 做法 |
|---|---|
| 快捷键最好在设置里也可以配置，Ctrl+Shift+A 和微信截图键冲突了；setting 专开一栏 | 设置文件加一个表 `[keys]`，10 项，每个动作一项 |
| 标签栏自动收起，在 mac 下只有全屏时候生效 | 0.5.0 里是有意限制的（系统的红绿灯按钮画在标签栏里）。改成：标签栏收起时把三个按钮一起藏起来，出现时再显示 |
| mac 终端全屏时候，左边总是有一段留空，非全屏时候那里是缩放关闭窗口的按钮 | 修：全屏时不留那 76 像素 |
| 主题加上 paper-dark 和 paper，keepane 里就是 | 颜色取自 keepane 的 `themes/paper-dark.conf` 和 `themes/paper.conf` |

## 按键：`[keys]`

| 设置项 | Windows 和 Linux 的默认 | macOS 的默认 |
|---|---|---|
| `keys.new_tab` | `ctrl-shift-t` | `cmd-t` |
| `keys.close_tab` | `ctrl-shift-w` | `cmd-w` |
| `keys.next_tab` | `ctrl-tab` | `ctrl-tab` |
| `keys.previous_tab` | `ctrl-shift-tab` | `ctrl-shift-tab` |
| `keys.copy` | `ctrl-shift-c` | `cmd-c` |
| `keys.paste` | `ctrl-shift-v`、`shift-insert` | `cmd-v` |
| `keys.agents` | `ctrl-shift-a` | `cmd-shift-a` |
| `keys.tab_bar` | `ctrl-shift-b` | `cmd-shift-b` |
| `keys.full_screen` | `f11`、`alt-enter` | `cmd-enter`、`ctrl-cmd-f` |
| `keys.explain` | `ctrl-shift-e` | `ctrl-shift-e` |

用香农六法得出的决定：

| 方法 | 结论 |
|---|---|
| 简化 | 一个动作对应一个键的列表。空列表就是没有键。不做"一个键做几件事"、不做按上下文的键 |
| 相似问题 | Windows Terminal、WezTerm、kitty 都是"动作 = 键"的表。键的写法照 GPUI 的键表（`ctrl-shift-t`），窗口直接拿去用，不用再转换 |
| 换说法 | "和微信冲突"不是 UnifyTerm 收到键后做错了，是微信注册了全系统的键，UnifyTerm 根本收不到。所以解法只能是换键，不是抢键 |
| 推广 | 原来"解释"（Ctrl-Shift-E）是在标签页里直接认键的，和别的窗口键不是一条路。这次改成同一条路（一个动作，由键表绑定），10 个动作一种机制 |
| 结构分析 | 三件事分开：键怎么写（`chord.rs`，纯文字处理，设置库里）；哪个动作有哪些键（`KeysConfig`）；窗口怎么绑定（`key_bindings`，改了就清掉重绑） |
| 逆向 | 最坏的结果是把键配错，再也打不了字或打不开设置。所以：会打出字的键（字母、数字、符号）必须带 Ctrl、Alt 或 Cmd；Enter、Tab、Esc、方向键单独或只带 Shift 一律拒绝；一个键不能给两个动作；`ut setting` 是在终端里运行的命令，键配错了仍然能用它改回来 |

规则：

- 写法宽松读入：`Ctrl+Shift+G`、`shift-ctrl-g` 都认；统一存成 `ctrl-shift-g`（顺序 `ctrl-alt-cmd-shift`，与 GPUI 自己写的顺序相同）。设置文件里手写的必须是统一写法，否则报错并给出该怎么写。
- 改了立即生效：窗口读到新设置后清掉键表重绑。
- 列表和弹框里的键（上下、Tab、Enter、Esc、Ctrl-N、Ctrl-P）不可配置：它们只在列表开着时起作用，不和别的程序冲突。

## paper 两套主题

| | paper-dark | paper |
|---|---|---|
| 前景 / 背景 | `#cecabe` / `#171716` | `#33302a` / `#f0e7d2` |
| 标签栏（keepane 的状态栏底色） | `#121212` | `#e3d8bf` |
| 列表的边线（keepane 的 pane 边线） | `#666664` | `#8a8172` |
| 暗字 | `#87837b` | `#635b4e` |
| 光标和强调色（keepane 的朱砂） | `#dd625e` | `#a53736` |
| 16 色 | keepane 的 `pane-colours` | 同左 |
| 选区 | `#3c3b38` | `#cac2b0` |

选区的颜色 keepane 没有定义，是这里算的（背景往前景走五分之一）。`panel`、`well`、`rule` 没给，按前景和背景算。这两套没有侧栏的图。

顺带改的一处遗漏：主题文件省掉 `cursor` 或 `selection` 时，原来会落到内置主题（Tokyo Night）的值上，浅色主题会出现深色主题的蓝色选区。个性（只有主题的缺省值这一处）；边界遗漏。改成：光标色用主题自己的前景色，选区从主题自己的前景和背景算。内置的九套都自己给了这两个颜色，不受影响。

## macOS 上的两处

**全屏时左边留空。** 标签栏左边给系统的三个按钮留了 76 像素，全屏时系统把按钮收起来了，空位还在。从 0.2.3 加全屏起就有。个性（只有这一处留位）；边界遗漏（没考虑全屏）。全屏时改成和别的系统一样，从 12 像素处开始。

**窗口模式下标签栏收不起来。** 做法：标签栏收起时，对系统的三个按钮各调一次 `setHidden:`，出现时再调回来。全屏时不动它们（系统自己管）。设完后读回 `isHidden`，写进日志（`UNIFYTERM_LOG=info` 时才记）。

## 在用户的 Mac 上查出来的

用 keepane 把仓库送到那台 Mac 的临时目录，编译、跑测试、启动窗口。那台 Mac 没有完整的 Xcode（缺 `metal` 编译器），GPUI 默认的构建方式过不了；用 GPUI 的 `runtime_shaders` 开关（运行时编译着色器）可以构建和运行。

| 发现 | 定性 | 改法 |
|---|---|---|
| macOS 上 4 个测试失败，窗口开起来带一行"设置没有采用" | macOS 的默认键 `cmd-shift-a` 与统一写法的顺序（当时是 `ctrl-alt-shift-cmd`）不一致，默认设置自己被判成写法不对。个性：一张表和一个函数对不上。更深一层是结构问题：另一个系统的默认值，在本系统上没有任何测试读过 | 顺序改成 `ctrl-alt-cmd-shift`；加一条在每个系统上都读两张默认表的测试 |
| macOS 的 clippy 报一个 `unsafe` 块里有 4 个操作 | 这条规则本机的代码没碰到过（Windows 那段只有 1 个操作） | 每个调用一个块 |

看不到的：那台 Mac 的屏幕取不到画面。按钮是否真的看不见、位置对不对，只能由用户看。

## 以后可加的

- `ut setting` 里按一下键就录入（现在要打字 `ctrl-alt-g`）。
- 按键冲突的提示：UnifyTerm 没法知道别的程序占了哪个全系统的键。
