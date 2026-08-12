---
title: 安装
---

# 安装

从 [GitHub Releases](https://github.com/Gaoshu705/QzoneArchive/releases) 下载与设备对应的最新版本。

## Windows

下载 `.exe` 安装程序并完成安装。系统需要 Windows 10 或更新版本，通常已包含 WebView2。

## macOS

根据设备下载 Intel 或 Apple Silicon 对应的 `.dmg`。首次启动时，系统可能要求在“隐私与安全性”中确认打开来源未知的应用。

## Android

下载 `.apk`，允许浏览器或文件管理器安装应用后按提示完成安装。请仅从项目 Releases 页面获取安装包。

## Linux

每个 Release 提供三种 Linux 安装包（具体文件名以 Release 页面实际下载到的为准，下面用 `*.ext` 通配符代替）：

- `.deb`：适合 Debian、Ubuntu 及其衍生发行版
- `.rpm`：适合 Fedora、openSUSE、RHEL 系发行版
- `.AppImage`：适合大多数桌面发行版，包括无法直接使用 `.deb`/`.rpm` 的发行版

### Debian / Ubuntu

```bash
sudo apt install ./*.deb
```

也可以使用 `dpkg`：

```bash
sudo dpkg -i *.deb
```

> **`apt` 与 `dpkg` 的区别**：
> - `apt install`：会自动联网补齐依赖，日常推荐使用；
> - `dpkg -i`：只安装包本身，不处理依赖。如果提示缺少依赖，先执行下面的命令补装：
>
> ```bash
> sudo apt-get install -f
> ```

卸载：

```bash
sudo apt remove qzonearchive
```

> 如果提示找不到包，可用 `dpkg -l | grep qzonearchive` 确认实际安装的包名（一般为 `qzonearchive`）。

### Fedora / openSUSE / RHEL 系

```bash
sudo rpm -i *.rpm
```

卸载：

```bash
sudo rpm -e qzonearchive
```

> 如果提示找不到包，可用 `rpm -qa | grep qzonearchive` 确认实际安装的包名（一般为 `qzonearchive`）。

### 通用 AppImage（适用于其他 Linux 发行版）

```bash
chmod +x *.AppImage
./*.AppImage
```

> 如果提示 `libfuse.so.2` 相关错误（常见于 Ubuntu 24.04 及以上），先安装 FUSE 依赖，或改用免 FUSE 的解包运行方式：
> ```bash
> sudo apt install libfuse2   # Debian / Ubuntu
> ./*.AppImage --appimage-extract-and-run
> ```

AppImage 是免安装的绿色软件，删除文件即可完成卸载。如果桌面环境没有自动集成应用菜单，可以自行创建 `.desktop` 文件，也可以直接把 AppImage 放到本地路径手动启动。

### NixOS

每个 Release 提供 NixOS 二进制包 `qzonearchive-nixos-x86_64-linux.tar.gz`，内含预构建的完整 Nix store closure 与一键安装脚本，适用于 NixOS 及其他使用 Nix 的 x86_64 Linux 系统。**安装只需一条命令**：

```bash
tar -xzf qzonearchive-nixos-x86_64-linux.tar.gz && ./install.sh
```

安装脚本会把依赖导入 `/nix/store` 并安装到当前用户 profile。提示输入密码时输入当前用户密码即可（与 `apt`/`dpkg` 相同）。完成后在终端运行：

```bash
qzonearchive
```

> 注意：请运行包内 `./install.sh` 完成安装，不要手动对解压目录执行 `nix profile install` 或直接运行二进制——应用依赖的 store 路径必须先导入。

NixOS 用户**无需 AppImage / deb / rpm**，请直接使用上方的一键安装包。

项目提供源码构建用的 Nix Flake。NixOS 用户可以进入项目目录执行：

```bash
nix build
./result/bin/qzonearchive
```

也可以安装到当前用户 profile：

```bash
nix profile install .#qzonearchive
```

Linux 用户安装 QQ 客户端时，请按你自己发行版的要求选择 QQ 官方提供的 deb、rpm 或 Flatpak 版本；空间归档本身不绑定或内置 QQ 客户端，只需要登录后扫描 QQ 空间的二维码即可使用。

从源码构建 Linux 版本可参考[开发](../development/)。
