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
    skip: "跳到正文",
    "nav.label": "本页目录",
    "nav.commands": "命令",
    "nav.history": "历史",
    "nav.media": "图片",
    "nav.help": "帮助",
    "nav.agents": "Agent",
    "nav.observations": "局限",
    "nav.install": "安装",
    "theme.label": "切换浅色和深色",
    "hero.alt": "Windows 上的 UnifyTerm，PowerShell 里：cat Cargo.toml | grep -n version 输出带行号的三行；echo '[1,2,3]' | jq add 输出 6；ut view unifyterm.png 把 logo 作为图片显示在终端里。",
    "stamp.seal": "印章：UnifyTerm 0.1.0，三个系统通用",
    "hero.h1": "一个终端，Windows、macOS、Linux 都认。",
    "hero.lede":
      "换一台机器，命令就跟着变。UnifyTerm 让它们保持一样：它运行你自己的 shell，并把 <code>cat</code>、<code>grep</code>、<code>jq</code> 等一百来个命令带到每个系统上，Windows 也不例外。",
    "field.type": "带窗口的终端",
    "field.shell": "用你自己的：PowerShell、cmd、bash、zsh、fish",
    "field.commands": "Windows 上 99 个，Linux 上 114 个",
    "field.issued": "用 Rust 写，MIT 许可",
    "entries.label": "适用的系统",
    "entry.windows": "10（1809）及以上",
    "entry.macos": "11 及以上",
    "entry.linux": "x86_64，Vulkan",
    "mrz.label": "安装方式",
    "mrz.copy": "复制",
    "mrz.copied": "已复制",
    "mrz.note.brew": "macOS 和 Linux，用 Homebrew。另外三种都是一行命令，不需要管理员权限。",
    "mrz.note.scoop": "Windows，用 Scoop。不需要管理员权限。",
    "mrz.note.curl": "macOS 和 Linux。只为你自己安装，不需要 root；安装前会校验 SHA-256。",
    "mrz.note.irm": "Windows，在 PowerShell 里运行。只为你自己安装；安装前会校验 SHA-256。",
    "commands.h": "过哪道关，命令都一样",
    "commands.p1":
      "PowerShell 里叫 <code>Get-Content</code>，bash 里叫 <code>cat</code>，而你的手只记得 <code>cat</code>。UnifyTerm 把 Unix 命令做成一个程序 <code>ut</code>，并排在 PowerShell 的别名前面。这样 <code>cat</code>、<code>ls</code>、<code>grep</code>、<code>find</code>、<code>sed</code>、<code>jq</code> 在三个系统上是同一个意思。",
    "commands.p2": "路径一律用 <code>/</code> 显示。通配符按 Unix shell 的规则展开，加了引号就原样保留。不用装 WSL，不用装 Git Bash，别的什么都不用装。",
    "commands.all": "Windows 上的全部 99 个命令",
    "commands.alt": "Windows 的 PowerShell 里，cat、grep、jq、ls、wc 和在 Linux 上一样运行。",
    "stamp.commands": "印章：Windows 上 99 个命令",
    "history.h": "边打字边查的历史",
    "history.p1":
      "shell 运行过的每条命令都存在一个很小的 SQLite 文件里，连同它所在的目录、结果和耗时。开始打字，匹配的命令就列在输入行下面。<kbd>Ctrl</kbd>+<kbd>N</kbd> 往下选，<kbd>Enter</kbd> 把它填到输入行上，再按一次 <kbd>Enter</kbd> 才会运行。",
    "history.p2": "在当前目录运行过的命令排得靠前，失败过的排得靠后。看起来像密码、令牌、密钥的内容不会被记下来。把这个文件拷到另一台机器上照样能用：Windows 上生成的历史，在 Linux 上读出来一模一样。",
    "history.alt": "输入 git 后，输入行下面列出三条以前用过的 git 命令，第二条被选中。",
    "stamp.history": "印章：在 10 万条命令里搜索一次不到 5 毫秒（95 分位）",
    "media.h": "图片、视频、markdown，就在文字旁边",
    "media.p1": "<code>ut view photo.png</code> 把图片显示在终端里。它跟着上面的文字一起滚动，清屏时一起消失。<code>ut view clip.mp4</code> 用 ffmpeg 播放视频，没有声音，按 <kbd>q</kbd> 停止。",
    "media.p2": "<code>ut md README.md</code> 把 markdown 排好版，表格、列表、链接都是真的。在别的终端里，<code>ut view</code> 会选那个终端能做到的最好方式；输出接到管道时，就退到纯字符。",
    "media.alt": "ut view 在终端里显示 UnifyTerm 的 logo；ut md 显示一个 markdown 文件，有标题、列表和表格。",
    "stamp.media": "印章：ut view 和 ut md",
    "help.h": "问一问这条命令是什么意思",
    "help.p1":
      "命令敲错了，它会给出一条改好的。这个猜测是在你的机器上做的，依据是它能找到的命令，不需要模型。在设置里指定一个模型之后，<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> 会把输入行上的命令逐段解释，打字时命令的后半段会以灰色显示。",
    "help.p2": "模型的帮助默认是关的：要你自己指定模型，并在窗口里同意一次才会开。同意之前，它会先告诉你要发送什么；密码、令牌、密钥会先被替换掉。你不同意，就什么都不发送。",
    "help.alt": "在 tar -czf site.tgz public 下面，一个框逐行解释 tar、-czf、site.tgz 和 public。",
    "stamp.help": "印章：你不同意，就什么都不发送",
    "agents.h": "给你用的 agent",
    "agents.p1":
      "<code>ut mcp</code> 给编程 agent 四个工具：搜索历史、列出最近的命令、列出 <code>ut</code> 有哪些命令、读某个命令的帮助。它只读，不运行任何东西。让 Claude Code 用上它：<code>claude mcp add unifyterm -- ut mcp</code>。",
    "agents.p2":
      "一份 skill 文件告诉 agent 这里什么能用，它就不用再猜 Windows 上有没有 <code>grep</code>。如果你用 <a href=\"https://github.com/newdee/keepane\" rel=\"noopener\">keepane</a>，按 <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>A</kbd> 可以在终端旁边看到它管理的 agent：模型、费用、上下文、内存。",
    "agents.alt": "终端右侧的侧栏列出两个 agent，以及各自的模型、窗格、费用和内存。",
    "stamp.agents": "印章：MCP，四个工具，只读",
    "film.h": "看它运行 46 秒",
    "film.p": "这段视频是从程序本身录下来的，每按一个键截一张图。里面没有任何手画的画面。",
    "film.link": "视频（MP4，0.4 MB）",
    "obs.h": "备注",
    "obs.lede": "还没做到的事写在这里，免得你用了才发现。",
    "obs.1": "<b>安装包还没有签名。</b>第一次启动前，Windows SmartScreen 和 macOS Gatekeeper 各会问一次。",
    "obs.2": "<b>macOS 版还没有人手工用过。</b>它在 CI 的 Mac 上构建、测试、启动过，仅此而已。",
    "obs.3": "<b>视频没有声音</b>，并且需要装 ffmpeg。",
    "obs.4": "<b>PowerShell 自带的 Tab 补全仍然输出 <code>\\</code> 路径。</b>命令打印出来的路径用的是 <code>/</code>。",
    "obs.5": "<b>它只显示用 kitty 协议发来的图片。</b>Sixel 和 iTerm2 格式是给 <code>ut view</code> 在别的终端里用的。",
    "obs.6": "<b>这是 0.1.0 版。</b>它背后的检查都在仓库里：207 项测试，另有 110 项是驱动真实窗口做的。",
    "install.h": "入境",
    "install.unix": "macOS 和 Linux",
    "install.unix.or": "或者一行命令，只为你自己安装，不需要 root：",
    "install.win.or": "或者一行命令，只为你自己安装，不需要管理员权限：",
    "install.packages":
      "安装包在<a href=\"https://github.com/newdee/unifyterm/releases\" rel=\"noopener\">发布页</a>，每个文件旁边都有它的 SHA-256：Windows 有 MSI 和 zip，两种 Mac 各有一个 .dmg，Linux 有 .deb 和 AppImage。两个一行安装脚本在安装前都会校验这个 SHA-256。",
    "install.source":
      "源码在 <a href=\"https://github.com/newdee/unifyterm\" rel=\"noopener\">github.com/newdee/unifyterm</a>，MIT 许可。设置项、快捷键和环境变量见 <a href=\"https://github.com/newdee/unifyterm#readme\" rel=\"noopener\">README</a>。",
    "foot.by":
      "作者 <a href=\"https://github.com/newdee\" rel=\"noopener\">newdee</a>。窗格、agent 和它们之间的消息是 <a href=\"https://github.com/newdee/keepane\" rel=\"noopener\">keepane</a> 的事；UnifyTerm 是它们运行所在的那个窗口。",
  };
  var en = {};
  var notes = {
    brew: "mrz.note.brew",
    scoop: "mrz.note.scoop",
    curl: "mrz.note.curl",
    irm: "mrz.note.irm",
  };
  en["mrz.note.scoop"] = "Windows, with Scoop. No administrator rights.";
  en["mrz.note.curl"] = "macOS and Linux. For you alone, no root; it checks the SHA-256 before it installs.";
  en["mrz.note.irm"] = "Windows, in PowerShell. For you alone; it checks the SHA-256 before it installs.";
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
    document.title = chinese ? "UnifyTerm · 一个终端，Windows、macOS、Linux 都认" : "UnifyTerm · one terminal, valid on Windows, macOS and Linux";
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
