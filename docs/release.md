# 发布清单

推送、打标签、发布、签名，以及提交到 winget，都会把内容发到外部，每一步都要用户来做或确认。

## 0. 发布前

1. 运行 `pwsh tools/round.ps1 -Profile release`，最后一行是 `0 failed`。
2. 确认 `docs/verification-log.md` 里最新阶段有连续三轮干净的记录。
3. 确认版本号：`Cargo.toml` 的 `[workspace.package] version`。`unifyterm --version` 和 `ut --version` 都显示它和提交号。
4. 推送到 `main`，等 `.github/workflows/ci.yml` 在三个平台都通过。macOS 的"启动窗口"一步允许失败（`continue-on-error`）；失败时把原因写进验收记录，并在发布说明里写"macOS 未手工验证"。

## 1. 打标签：其余由工作流完成

1. 运行 `git tag v<版本>`，然后 `git push origin v<版本>`。
2. `.github/workflows/release.yml` 会做下面这些事：
   - 等 CI 在这个提交上通过，不通过就不发布；
   - 构建 Windows 的 zip 和 MSI，Linux 的 `.tar.gz`、`.deb`、AppImage，macOS 两种架构的 `.tar.gz` 和 `.dmg`，每个文件旁边放一个 `.sha256`；
   - 创建 GitHub Release；
   - 用刚构建的文件生成 scoop 和 winget 清单（`tools/package-manifests.ps1`），并提交回 `main`。按地址装过的（`scoop install <raw 地址>`）从此更新到这个版本。
3. Scoop bucket：仓库 `newdee/scoop-bucket` 的工作流 `unifyterm.yml` 每 6 小时从 `main` 复制一次清单，在 Windows 上用 Scoop 装好、运行、卸载通过后才提交。`scoop install unifyterm` 从此装的就是这个版本。想马上更新，就在那个仓库里手动运行一次这个工作流（`gh workflow run unifyterm.yml --repo newdee/scoop-bucket`）。
4. Homebrew：tap 仓库 `newdee/homebrew-tap` 的工作流 `unifyterm.yml` 每 6 小时跟一次最新版本，在 macOS 和 Linux 上装好并测试通过后，才提交新的 formula。想马上更新，就在 tap 仓库里手动运行一次这个工作流。
5. `install.sh` 和 `install.ps1` 总是安装最新的 Release，不需要改。
6. 网页：`site/` 有改动并推送到 `main` 时，`.github/workflows/pages.yml` 把它发布到 https://dfine.tech/unifyterm/ 。版本号和命令数量写在网页里，发新版本时要改；`pwsh tools/site-check.ps1` 会核对命令数量和安装命令。

## 2. 检查发布出来的东西

1. Windows：
   - 在干净的机器或 Windows Sandbox 里安装 MSI，检查开始菜单有 UnifyTerm，启动后 `cat --version` 能运行。
   - 卸载后 `C:\Program Files\unifyterm` 不存在；`%APPDATA%\unifyterm` 和 `%LOCALAPPDATA%\unifyterm` 保留。
   - 用 scoop 和 `install.ps1` 各装一次，运行 `unifyterm --version`。
2. Linux：`sudo apt install ./unifyterm_*.deb`；`unifyterm --version`、`ut --list` 能运行；`sudo apt remove unifyterm`。再用 `install.sh` 装一次。
3. macOS：`brew install newdee/tap/unifyterm`；打开 `.dmg` 拖进"应用程序"，右键 → 打开。

## 3. 签名（可选，需要证书）

- Windows：用代码签名证书给 `unifyterm.exe`、`ut.exe` 和 MSI 签名（`signtool sign /fd sha256 /tr <时间戳服务器> /td sha256 …`）。没有签名时 SmartScreen 会警告一次。
- macOS：用 Developer ID 签名 `.app`（`codesign --deep --options runtime`），再用 `xcrun notarytool submit` 公证，`xcrun stapler staple`。没有签名时 Gatekeeper 会拦一次。

## 4. winget

工作流已经把清单写进 `packaging/windows/winget/`。把这三个 YAML 提交到 https://github.com/microsoft/winget-pkgs 的 `manifests/n/newdee/unifyterm/<版本>/`（需要用户的账号）。

## 5. 发布后

1. 在三个平台各装一次正式包，运行 `unifyterm --version`，确认版本和提交号。
2. 在 `docs/roadmap.md` 里记下发布的版本和日期。
