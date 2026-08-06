# Tauri v2 iOS 发布研究

研究日期：2026-08-06。本文只依据 Tauri 官方文档、Tauri 官方仓库及 Apple 官方文档。

## 1. `bundle.icon` 与 iOS AppIcon

结论：Tauri v2 的 `tauri.conf.json > bundle.icon` 只是跨平台 bundler 的“应用图标列表”配置；当前 iOS CLI 构建路径**不会**在 `tauri ios build` 时读取该字段并生成 `AppIcon`。不要把它当作 iOS AppIcon 的输入。

- iOS 项目由 `tauri ios init` 模板生成，模板包含 `Assets.xcassets/AppIcon.appiconset`，并将 `Assets.xcassets` 作为 Xcode target source。
- 使用 `tauri icon <方形 PNG 或 SVG>` 生成 iOS 图标。若 `src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset` 已存在（即已初始化 iOS），Tauri CLI 会把完整尺寸集写到该目录；否则写到 `src-tauri/icons/ios`。`--ios-color` 可指定 iOS 图标的透明区域背景色；非方形输入须显式使用 `--fit cover` 或 `--fit contain`。
- `AppIcon.appiconset/Contents.json` 声明 iPhone、iPad 和 `ios-marketing` 的尺寸槽位，Xcode 编译 asset catalog 后在产物中使用。项目的 iOS `project.yml` 已包含 `Assets.xcassets`，无需通过 `bundle.resources` 再复制图标。
- 若需要自定义 asset catalog，可使用 Tauri iOS 配置的 `assetCatalogs`；但其用途是额外 catalog，不会改变上述 `bundle.icon` 与 AppIcon 无自动关联的事实。

来源：

- [Tauri 配置参考：`bundle.icon`](https://v2.tauri.app/reference/config/#icon)
- [Tauri CLI `icon.rs`：iOS 输出目录、尺寸及背景色处理](https://github.com/tauri-apps/tauri/blob/c0bd0d5a61eedba5c4783add24455c5028c6f390/crates/tauri-cli/src/icon.rs#L68-L108)
- [Tauri CLI `icon.rs`：检测 `gen/apple` 并写入 AppIcon](https://github.com/tauri-apps/tauri/blob/c0bd0d5a61eedba5c4783add24455c5028c6f390/crates/tauri-cli/src/icon.rs#L842-L868)
- [Tauri iOS 模板：Xcode target 引入 `Assets.xcassets`](https://github.com/tauri-apps/tauri/blob/c0bd0d5a61eedba5c4783add24455c5028c6f390/crates/tauri-cli/templates/mobile/ios/project.yml#L30-L45)
- [Tauri iOS AppIcon 槽位清单](https://github.com/tauri-apps/tauri/blob/c0bd0d5a61eedba5c4783add24455c5028c6f390/crates/tauri-cli/templates/mobile/ios/Assets.xcassets/AppIcon.appiconset/Contents.json)
- [Apple：App icons](https://developer.apple.com/design/human-interface-guidelines/app-icons)

## 2. 未签名 IPA、真机与无签名 CI

结论：**未签名 IPA 不能作为可在普通未越狱真机上安装并打开的交付物。**iOS 真机代码必须由可接受的签名和匹配的描述文件授权；开发安装需要开发证书、App ID、已注册设备和开发描述文件，Ad Hoc 安装需要分发证书、App ID、已注册设备和 Ad Hoc 描述文件。

- `tauri ios build --no-sign` 会跳过签名，仍将 archive 内的 `.app` 直接 zip 为 `Payload/*.app` 的 IPA。因此“CI 成功并产生 `.ipa`”只表示打包成功，不表示该 IPA 可安装或可运行。
- Tauri 源码对默认构建会启用 `allow_provisioning_updates`；`--no-sign` 则调用 `skip_codesign`。所以 CI 未提供签名资产时，是否失败还取决于 Xcode 能否通过可用登录态/自动签名获取资产；在干净、无 Apple 登录态的 runner 上，不应依赖这种行为。明确传 `--no-sign` 时可预期得到未签名 IPA，而不是可安装 IPA。
- `--archive-only` 只输出 `.xcarchive`，不产 IPA。`--export-method debugging`、`release-testing`、`app-store-connect` 分别对应开发、Ad Hoc、App Store Connect 的导出语义（Xcode 15.4 前使用 `development`、`ad-hoc`、`app-store` 名称）。

来源：

- [Tauri iOS build 参数和 `--no-sign` 行为](https://github.com/tauri-apps/tauri/blob/c0bd0d5a61eedba5c4783add24455c5028c6f390/crates/tauri-cli/src/mobile/ios/build.rs#L42-L105)
- [Tauri iOS build：跳过签名、archive 与未签名 IPA 打包逻辑](https://github.com/tauri-apps/tauri/blob/c0bd0d5a61eedba5c4783add24455c5028c6f390/crates/tauri-cli/src/mobile/ios/build.rs#L432-L571)
- [Apple：创建开发描述文件的前置条件](https://developer.apple.com/help/account/provisioning-profiles/create-a-development-provisioning-profile)
- [Apple：Ad Hoc 描述文件的前置条件与用途](https://developer.apple.com/help/account/provisioning-profiles/create-an-ad-hoc-provisioning-profile)
- [Apple：注册设备是开发或 Ad Hoc 描述文件的前提](https://developer.apple.com/help/account/devices/register-a-single-device)

## 3. GitHub Actions 生成可安装 IPA

### 所需资产

适用于给已注册设备安装的 release IPA（Ad Hoc）：

1. macOS runner 和匹配版本的 Xcode；iOS 不能在 Linux 或 Windows runner 上完成 Xcode archive/export。
2. Apple Developer Program 团队、显式 App ID，且 App ID 的 Bundle ID 必须与 Tauri `identifier` 相同；所需 capabilities/entitlements 必须先在该 App ID 启用。
3. `Apple Distribution` 证书连同私钥导出的 `.p12`，以及其导出密码。
4. 与该证书、App ID、目标设备 UDID 匹配的 Ad Hoc `.mobileprovision`。变更设备或 capabilities 后应重新生成 profile。
5. Tauri 的手动签名输入：`IOS_CERTIFICATE`（P12 的 base64 内容）、`IOS_CERTIFICATE_PASSWORD`、`IOS_MOBILE_PROVISION`（profile 的 base64 内容）；并设置 `bundle.iOS.developmentTeam` 或 `APPLE_DEVELOPMENT_TEAM`。执行 `tauri ios build --export-method release-testing`。

面向 App Store Connect/TestFlight 时，改用 App Store Connect profile 和同一类 Apple Distribution 证书，执行 `--export-method app-store-connect`，然后上传；这类 IPA 不是给设备侧载的 Ad Hoc 包。Apple 官方说明 App Store Connect profile 必须使用显式 App ID，且包含一个分发证书。

可选的自动签名路径：Tauri 会在同时存在 `APPLE_API_KEY`、`APPLE_API_ISSUER`、`APPLE_API_KEY_PATH` 时把它们传给 Xcode 以进行签名资产操作；三个变量必须同时存在。此路径仍要求 Apple 账户具备相应权限，且会扩大 CI 凭据权限范围。

### 安全做法

- 将 P12、P12 密码、mobileprovision 和 API 私钥存为 GitHub Actions secrets 或受保护 Environment secrets，绝不提交到仓库、构建产物或日志；二进制文件可 base64 后保存，但 base64 不是加密。
- 仅在受保护的 tag/发布分支和需审批的 production environment 执行签名 job；不要在来自 fork 的 PR、Dependabot 事件或不受信任的 `pull_request_target` 上暴露签名 secrets。GitHub 默认不会把普通 secrets 传给 fork 触发的 workflow。
- 使用最小权限：专用发布证书/专用 API key，限制 secret 的组织与仓库访问范围；定期检查 profile、设备和证书有效期，泄露时立即撤销并重新签发。Apple 明确将 Apple 账户凭据和分发证书视为敏感资产，禁止在组织外共享证书。
- 在 job 结束时删除临时 keychain、解码后的 P12 与 profile；不要把它们上传为 artifact。向命令传密钥优先使用环境变量或标准输入，避免将 secret 放到命令行或日志。

来源：

- [Apple：注册 App ID，显式 ID 应与 Xcode target Bundle ID 一致](https://developer.apple.com/help/account/manage-identifiers/register-an-app-id)
- [Apple：证书类型、Apple Development 与 Apple Distribution 的用途及保护要求](https://developer.apple.com/help/account/certificates/certificates-overview)
- [Apple：Ad Hoc profile 选择分发证书与设备](https://developer.apple.com/help/account/provisioning-profiles/create-an-ad-hoc-provisioning-profile)
- [Apple：App Store Connect profile 的显式 ID 与单个分发证书要求](https://developer.apple.com/help/account/provisioning-profiles/create-an-app-store-provisioning-profile)
- [Tauri：从环境变量读取 P12 和 provisioning profile，并写入 Xcode 签名设置](https://github.com/tauri-apps/tauri/blob/c0bd0d5a61eedba5c4783add24455c5028c6f390/crates/tauri-cli/src/mobile/ios/mod.rs#L484-L516)
- [Tauri：Apple API key 三个环境变量必须成组提供](https://github.com/tauri-apps/tauri/blob/c0bd0d5a61eedba5c4783add24455c5028c6f390/crates/tauri-cli/src/mobile/ios/build.rs#L643-L660)
- [GitHub Actions：secrets 使用、fork 限制、日志遮蔽及避免命令行传 secret](https://docs.github.com/en/actions/security-for-github-actions/security-guides/using-secrets-in-github-actions)
- [Apple Developer Program：分发需要会员资格提供的 Certificates, Identifiers & Profiles、TestFlight 和 App Store Connect](https://developer.apple.com/help/account/membership/programs-overview)
