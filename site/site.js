// UnifyTerm's page: the light or dark page, the language, the command in the
// machine-readable band, and the stamps that land once. The page reads whole
// without any of it.
(function () {
  "use strict";
  var root = document.documentElement;
  root.classList.add("js");

  function keep(key, value) {
    try {
      localStorage.setItem(key, value);
    } catch (error) {
      // Private windows may refuse: the choice then lasts for this visit.
    }
  }

  // ---- light and dark -------------------------------------------------
  var themeButton = document.getElementById("theme");
  themeButton.addEventListener("click", function () {
    var next = root.dataset.theme === "dark" ? "light" : "dark";
    root.dataset.theme = next;
    keep("unifyterm-theme", next);
  });

  // ---- the two languages ----------------------------------------------
  // English is in the page. This is the Chinese for each marked element.
  var zh = {
    "skip": "跳到正文",
    "nav.label": "本页目录",
    "nav.commands": "命令",
    "nav.history": "历史",
    "nav.media": "图片",
    "nav.help": "辅助",
    "nav.agents": "Agent",
    "nav.observations": "限制",
    "nav.install": "安装",
    "theme.label": "切换浅色和深色",
    "hero.alt": "Windows 上的 UnifyTerm，PowerShell 里：cat Cargo.toml | grep -n version 输出带行号的三行；echo '[1,2,3]' | jq add 输出 6；ut view unifyterm.png 把 logo 作为图片显示在终端里。",
    "stamp.seal": "印章：UnifyTerm 0.1.1，三个系统通用",
    "hero.h1": "一个终端，运行在 Windows、macOS 和 Linux 上。",
    "hero.sub": "在不同操作系统上，使用相同的命令和操作习惯。",
    "hero.lede": "UnifyTerm 是一个跨平台终端应用。它保留系统原生 Shell，同时提供一套统一的 Unix 命令工具，包括 <code>cat</code>、<code>grep</code>、<code>sed</code>、<code>jq</code> 等。",
    "hero.lede2": "在 Windows 上，不需要安装 WSL 或 Git Bash，也能使用这些常见命令。",
    "field.type": "桌面终端应用",
    "field.shell": "PowerShell、cmd、bash、zsh、fish",
    "field.commands": "Windows 99 个，Linux 114 个",
    "field.repo": "GitHub 仓库",
    "entries.label": "适用的系统",
    "entry.windows": "10（1809）及以上",
    "entry.macos": "11 及以上",
    "entry.linux": "x86_64，Vulkan",
    "mrz.label": "安装方式",
    "mrz.copy": "复制",
    "mrz.copied": "已复制",
    "mrz.note.brew": "macOS 和 Linux，使用 Homebrew。",
    "mrz.note.scoop": "Windows，使用 Scoop。",
    "mrz.note.curl": "macOS 和 Linux，使用安装脚本。不需要 root 权限。",
    "mrz.note.irm": "Windows，使用 PowerShell 安装脚本。不需要管理员权限。",
    "commands.h": "统一的命令环境",
    "commands.p1": "Windows 和 Unix 系统在日常命令上存在不少差异。同一件事情，在不同系统上可能需要使用不同的命令。",
    "commands.p2": "UnifyTerm 提供了一套统一的 Unix 命令工具。无论是在 Windows、macOS 还是 Linux 上，都可以使用 <code>cat</code>、<code>ls</code>、<code>grep</code>、<code>find</code>、<code>sed</code>、<code>jq</code> 等常用命令。",
    "commands.p3": "同时处理不同平台的路径格式和通配符展开问题，让脚本和命令更容易在不同系统之间复用。",
    "commands.p4": "在 Windows 上，这些命令也可以正常使用，不需要额外配置一套 Unix Shell 环境。",
    "commands.p5": "目前 Windows 版本内置了 99 个命令，覆盖文件操作、文本处理、数据查询等常见场景。",
    "commands.all": "Windows 上的全部 99 个命令",
    "commands.alt": "Windows 的 PowerShell 里，cat、grep、jq、ls、wc 和在 Linux 上一样运行。",
    "stamp.commands": "印章：Windows 上 99 个命令",
    "history.h": "命令历史与智能搜索",
    "history.p1": "UnifyTerm 使用 SQLite 在本地保存命令历史。",
    "history.p2": "除了命令本身，还会记录执行目录、退出状态和执行时间。输入命令时，会根据当前目录和历史使用情况实时提供匹配结果。",
    "history.l1": "优先显示当前目录下使用过的命令。",
    "history.l2": "根据执行结果调整历史命令的排序。",
    "history.l3": "使用 <kbd>Ctrl</kbd>+<kbd>N</kbd> 浏览候选命令。",
    "history.l4": "按 <kbd>Enter</kbd> 选择，再次按下执行。",
    "history.l5": "对密码、Token、密钥等敏感信息进行过滤。",
    "history.p3": "历史数据保存在本地数据库中，可以在不同设备之间迁移。Windows 上积累的命令历史，也可以在 Linux 上继续使用。",
    "history.alt": "输入 git 后，输入行下面列出三条以前用过的 git 命令，第二条被选中。",
    "stamp.history": "印章：在 10 万条命令里搜索一次不到 5 毫秒（95 分位）",
    "media.h": "在终端中查看图片、视频和 Markdown",
    "media.p1": "终端不一定只能显示文本。",
    "media.p2": "UnifyTerm 提供了图片、视频和 Markdown 的查看能力，可以直接在终端中打开这些内容。",
    "media.h1": "图片和视频",
    "media.p3": "视频播放依赖 FFmpeg，目前不支持音频。",
    "media.p4": "支持在终端中渲染 Markdown 文档，包括标题、列表、表格和链接等内容。",
    "media.p5": "当前终端不支持图片显示时，会在可用的情况下回退到文本输出。",
    "media.alt": "ut view 在终端里显示 UnifyTerm 的 logo；ut md 显示一个 markdown 文件，有标题、列表和表格。",
    "stamp.media": "印章：ut view 和 ut md",
    "help.h": "命令辅助",
    "help.p1": "UnifyTerm 提供本地命令纠错和基于大语言模型的命令辅助。",
    "help.h1": "本地命令纠错",
    "help.p2": "输入错误的命令时，会检查可用命令并给出修正建议。这部分在本地完成，不需要联网，也不依赖 AI 模型。",
    "help.h2": "AI 命令解释",
    "help.p3": "配置大语言模型后，可以通过 <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> 获取命令及其参数的解释，也可以在输入过程中获得命令补全建议。",
    "help.p4": "AI 功能默认关闭。",
    "help.p5": "启用之前，UnifyTerm 会展示即将共享的信息，并请你确认一次。每次请求都会先过滤密码、Token 和密钥等敏感内容。是否启用 AI 辅助，由用户决定。",
    "help.alt": "在 tar -czf site.tgz public 下面，一个框逐行解释 tar、-czf、site.tgz 和 public。",
    "stamp.help": "印章：你不同意，就什么都不发送",
    "agents.h": "面向 AI Agent 的终端集成",
    "agents.p1": "UnifyTerm 提供 MCP Server 和 Skill，方便 AI Coding Agent 获取终端环境信息和使用命令。",
    "agents.p2": "MCP Server 目前提供四个只读工具：",
    "agents.l1": "查询命令历史。",
    "agents.l2": "查看最近执行的命令。",
    "agents.l3": "获取可用命令列表。",
    "agents.l4": "查询命令帮助信息。",
    "agents.p3": "MCP 工具不提供命令执行能力，Agent 不能通过这些接口直接执行 Shell 命令。",
    "agents.p4": "例如，在 Claude Code 中添加 UnifyTerm：",
    "agents.p5": "配套的 Skill 描述了可用命令及不同操作系统下的行为，帮助 Agent 在不同平台上使用一致的命令环境。",
    "agents.p6": "如果同时使用 <a href=\"https://github.com/newdee/keepane\" rel=\"noopener\">keepane</a>，还可以在终端中查看 Agent 的运行信息，包括模型、费用、上下文和内存占用等。",
    "agents.alt": "终端右侧的侧栏列出两个 agent，以及各自的模型、窗格、费用和内存。",
    "stamp.agents": "印章：MCP，四个工具，只读",
    "film.h": "实际运行效果",
    "film.p": "这里展示 UnifyTerm 在真实环境中的运行过程，包括命令执行、历史搜索、内容查看和 Agent 集成。",
    "film.link": "观看演示视频",
    "obs.h": "当前限制",
    "obs.lede": "UnifyTerm 目前处于 0.1.1 版本，仍有一些尚未完善的地方：",
    "obs.1": "Windows 和 macOS 版本尚未进行代码签名，首次运行可能出现 SmartScreen 或 Gatekeeper 安全提示。",
    "obs.2": "macOS 版本已通过 CI 构建和测试，但人工使用还很少。",
    "obs.3": "视频播放依赖 FFmpeg，目前不支持音频。",
    "obs.4": "PowerShell 原生 Tab 补全仍然使用 Windows 风格的路径分隔符。",
    "obs.5": "UnifyTerm 窗口内的图片渲染目前只支持 Kitty 图形协议；<code>ut view</code> 在其他终端中运行时，还可以输出 Sixel 和 iTerm2 格式。",
    "obs.tests": "目前项目包含 210 个测试用例，另有 112 个针对桌面应用的测试。",
    "install.h": "安装",
    "install.brew": "使用 Homebrew：",
    "install.curl": "或者使用安装脚本：",
    "install.scoop": "使用 Scoop：",
    "install.irm": "或者使用 PowerShell 安装脚本：",
    "install.packages": "也可以从 <a href=\"https://github.com/newdee/unifyterm/releases\" rel=\"noopener\">GitHub Releases</a> 下载预编译版本，包括 MSI、ZIP、DMG、DEB 和 AppImage。",
    "install.sha": "安装脚本会在安装前校验 SHA-256 文件摘要。",
    "install.source": "更多配置选项、环境变量和使用说明，请查看 <a href=\"https://github.com/newdee/unifyterm\" rel=\"noopener\">GitHub 仓库</a>。",
    "foot.by": "由 <a href=\"https://github.com/newdee\" rel=\"noopener\">newdee</a> 开发。",
    "foot.keepane": "UnifyTerm 负责统一终端环境和命令接口。如果需要管理终端 Pane、Agent 生命周期以及多个 Agent 之间的协作，可以了解另一个项目 <a href=\"https://github.com/newdee/keepane\" rel=\"noopener\">keepane</a>。",
  };
  var en = {};
  var notes = {
    brew: "mrz.note.brew",
    scoop: "mrz.note.scoop",
    curl: "mrz.note.curl",
    irm: "mrz.note.irm",
  };
  en["mrz.note.scoop"] = "Windows, with Scoop.";
  en["mrz.note.curl"] = "macOS and Linux, with the install script. No root required.";
  en["mrz.note.irm"] = "Windows, with the PowerShell install script. No administrator rights required.";
  en["mrz.copied"] = "Copied";

  var marked = document.querySelectorAll("[data-i18n]");
  var attributed = document.querySelectorAll("[data-i18n-attr]");
  // What the page says in English, taken once, to come back to.
  marked.forEach(function (element) {
    var key = element.dataset.i18n;
    if (!(key in en)) en[key] = element.innerHTML;
  });
  attributed.forEach(function (element) {
    element.dataset.i18nAttr.split(";").forEach(function (pair) {
      var parts = pair.split(":");
      en[parts[1]] = element.getAttribute(parts[0]);
    });
  });

  var langButton = document.getElementById("lang");
  function words(key) {
    return (root.dataset.lang === "zh" ? zh[key] : en[key]) || en[key];
  }
  function speak() {
    var chinese = root.dataset.lang === "zh";
    root.lang = chinese ? "zh-CN" : "en";
    marked.forEach(function (element) {
      element.innerHTML = words(element.dataset.i18n);
    });
    attributed.forEach(function (element) {
      element.dataset.i18nAttr.split(";").forEach(function (pair) {
        var parts = pair.split(":");
        element.setAttribute(parts[0], words(parts[1]));
      });
    });
    langButton.textContent = chinese ? "English" : "中文";
    langButton.setAttribute("aria-label", chinese ? "Switch to English" : "切换到中文");
    document.title = chinese ? "UnifyTerm · 一个终端，运行在 Windows、macOS 和 Linux 上" : "UnifyTerm · One terminal across Windows, macOS and Linux";
    showNote();
  }
  langButton.addEventListener("click", function () {
    var next = root.dataset.lang === "zh" ? "en" : "zh";
    if (next === "zh") root.dataset.lang = "zh";
    else delete root.dataset.lang;
    keep("unifyterm-lang", next);
    speak();
  });

  // ---- the machine-readable band: the command for this system ---------
  var tabs = Array.prototype.slice.call(document.querySelectorAll(".mrz__tabs [role=tab]"));
  var command = document.getElementById("command");
  var note = document.getElementById("mrz-note");
  var panel = document.getElementById("mrz-line");
  var copied = document.getElementById("copied");
  var chosen = "brew";

  function showNote() {
    note.dataset.i18n = notes[chosen];
    note.innerHTML = words(notes[chosen]);
  }
  function choose(tab, focus) {
    tabs.forEach(function (other) {
      var selected = other === tab;
      other.setAttribute("aria-selected", String(selected));
      other.tabIndex = selected ? 0 : -1;
    });
    chosen = tab.id.replace("tab-", "");
    command.textContent = tab.dataset.command;
    panel.setAttribute("aria-labelledby", tab.id);
    copied.textContent = "";
    showNote();
    if (focus) tab.focus();
  }
  tabs.forEach(function (tab, index) {
    tab.addEventListener("click", function () {
      choose(tab, false);
    });
    tab.addEventListener("keydown", function (event) {
      var step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
      if (!step) return;
      event.preventDefault();
      choose(tabs[(index + step + tabs.length) % tabs.length], true);
    });
  });
  // Windows visitors get scoop first; everyone else, brew.
  var platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  if (/win/i.test(platform)) choose(document.getElementById("tab-scoop"), false);

  document.getElementById("copy").addEventListener("click", function () {
    var text = command.textContent;
    var done = function () {
      copied.textContent = "";
      // A new element state, so that the stamp lands again on a second press.
      requestAnimationFrame(function () {
        copied.textContent = words("mrz.copied");
      });
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(done, function () {
        selectCommand();
      });
    } else {
      selectCommand();
    }
  });
  // Where the clipboard is not allowed, the command is selected: Ctrl+C takes it.
  function selectCommand() {
    var range = document.createRange();
    range.selectNodeContents(command);
    var selection = getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  }

  // ---- the stamps land once, as their page comes into view -------------
  var still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  var stamps = document.querySelectorAll(".stamp, .entry");
  if (!still && "IntersectionObserver" in window) {
    var seen = new IntersectionObserver(
      function (entries) {
        entries.forEach(function (entry) {
          if (!entry.isIntersecting) return;
          var stamp = entry.target;
          var wait = Number(stamp.dataset.wait || 0);
          setTimeout(function () {
            stamp.classList.remove("is-waiting");
          }, wait);
          seen.unobserve(stamp);
        });
      },
      { threshold: 0.6 }
    );
    stamps.forEach(function (stamp, index) {
      // What is on the screen at once is already stamped: only pages below wait.
      if (stamp.getBoundingClientRect().top < innerHeight * 0.9) return;
      stamp.classList.add("is-waiting");
      if (stamp.classList.contains("entry")) stamp.dataset.wait = String((index % 3) * 110);
      seen.observe(stamp);
    });
  }

  speak();
})();
