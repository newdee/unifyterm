# 发布清单

每一步都要用户来做或确认：推送、打标签、发布、签名、提交到 winget 和 Homebrew，都会把内容发到外部。

## 0. 发布前

1. 运行 `pwsh tools/round.ps1 -Profile release`，最后一行是 `0 failed`。
2. 确认 `docs/verification-log.md` 里最新阶段有连续三轮干净的记录。
3. 把所有文件里的 `OWNER` 换成真实的 GitHub 账号或组织（`grep -rn OWNER packaging README.md`）。
4. 确认版本号：`Cargo.toml` 的 `[workspace.package] version`。`uni-term --version` 和 `ut --version` 都显示它和提交号。

## 1. 推送并等 CI 全绿

1. 添加远程仓库并推送（需要用户的账号）。
2. 等 `.github/workflows/ci.yml` 在三个平台都通过。
3. macOS 的"启动窗口"一步允许失败（`continue-on-error`）。失败时把原因写进 `docs/verification-log.md`，并在发布说明里写"macOS 未手工验证"。

## 2. 下载 CI 的产物并检查

1. 从 CI 下载三个平台的 artifact，放进同一个目录。
2. Windows：在一台干净的机器或 Windows Sandbox 里安装 MSI。检查：开始菜单有 uni-term；启动后 `cat --version` 能运行；卸载后 `C:\Program Files\uni-term` 不存在；`%APPDATA%\uni-term` 和 `%LOCALAPPDATA%\uni-term` 保留。
3. Linux：`sudo apt install ./uni-term_*.deb`；`uni-term --version`、`ut --list` 能运行；`sudo apt remove uni-term`。
4. macOS：打开 `.dmg`，拖进"应用程序"，右键 → 打开。

## 3. 签名（可选，需要证书）

- Windows：用代码签名证书给 `uni-term.exe`、`ut.exe` 和 MSI 签名（`signtool sign /fd sha256 /tr <时间戳服务器> /td sha256 …`）。没有签名时 SmartScreen 会警告一次。
- macOS：用 Developer ID 签名 `.app`（`codesign --deep --options runtime`），再用 `xcrun notarytool submit` 公证，`xcrun stapler staple`。没有签名时 Gatekeeper 会拦一次。

## 4. 打标签、发布

1. `git tag v<版本>`，推送标签。
2. 在 GitHub 上新建 Release，附上 MSI、`.deb`、AppImage、两个 `.dmg`。
3. 发布说明里写明：验证过的平台和方式（见验收记录）、没有签名、已知限制（README 的 Known limits）。

## 5. winget 和 Homebrew

1. 运行 `pwsh tools/release.ps1 -Owner <账号> -Packages <产物目录>`。它把版本、地址、SHA-256 和 MSI 的 ProductCode 填进 `target/release-manifests/` 下的副本，不改 `packaging/` 里的模板。
2. winget：把三个 YAML 提交到 https://github.com/microsoft/winget-pkgs 的 `manifests/u/uni-term/uni-term/<版本>/`（需要用户的账号）。
3. Homebrew：把 `uni-term.rb` 放进用户自己的 tap（`homebrew-tap` 仓库的 `Casks/`）。

## 6. 发布后

1. 在三个平台各装一次正式包，运行 `uni-term --version`，确认版本和提交号。
2. 在 `docs/roadmap.md` 里记下发布的版本和日期。
