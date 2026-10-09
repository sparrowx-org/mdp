<div align="center">

# mdp

**macOS 多显示器布局管理 —— 把「每块屏放哪、哪块是主屏」存成方案，敲 `mdp` 选一下回车就切过去。**

[![CI](https://github.com/define9/mdp/actions/workflows/ci.yml/badge.svg)](https://github.com/define9/mdp/actions/workflows/ci.yml)
[![Release](https://github.com/define9/mdp/actions/workflows/release.yml/badge.svg)](https://github.com/define9/mdp/releases/latest)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![macOS](https://img.shields.io/badge/macOS-11%2B-42a5f5.svg)](https://www.apple.com/macos/)

</div>

```
$ mdp
 mdp · 选择方案
▌1 * work                 work
 2   swap                 外接为主屏，笔记本放右边
 3   movie

                          ★ External Display · 1920x1080
                            (0,0) = (0,0)
                            Built-in Display · 3024x1964
                            (1920,268) ← (1512,0)
```

按 `↓` 选一个方案，右侧实时显示每块屏会移到哪、往哪个方向移；按 `Enter` 立即切换。

---

## 特性

- **一键切布局** —— 无参数 `mdp` 打开选择器，方向键选、回车生效，没有多余步骤
- **连主屏一起存** —— 主屏 = 原点在 `(0,0)` 的那块屏，切方案时主屏跟着变，不用再去系统设置里点「主显示器」
- **零权限** —— 只用 CoreGraphics 的**公开 API**，不需要「辅助功能」「屏幕录制」授权
- **原子切换** —— 整个布局在一个事务里提交，中途失败自动回滚，不会留下半截布局
- **会说实话** —— macOS 会自作主张消除空隙和重叠，应用后 mdp 回读实际位置并明确告诉你哪块屏没落在期望点
- **认得出设备** —— 用持久 UUID 标识显示器，换扩展坞、插拔、重启都不会认错屏
- **脚本友好** —— 所有子命令支持 `--json`；非交互终端下 `mdp` 会提示改用 `mdp apply`

## 安装

### Homebrew（推荐）

```bash
brew tap define9/mdp
brew install mdp
```

一条命令版本（Homebrew 会自动 tap）：

```bash
brew install define9/mdp/mdp
```

### curl | sh

```bash
curl -fsSL https://raw.githubusercontent.com/define9/mdp/main/install.sh | sh
```

### 从源码

```bash
git clone https://github.com/define9/mdp.git
cd mdp
cargo install --path .
```

> 需要 Rust 1.85+。`cargo install mdp`（crates.io）待发布后可用。

## 快速开始

```bash
# 1. 把你现在的布局（双屏、外接在左、主屏是外接）存下来
$ mdp store work "外接为主屏，笔记本放右边"

# 2. 临时改成别的布局

# 3. 随时切回来
$ mdp
```

方案存在 `~/.config/mdp/solutions.toml`，纯文本，可以直接进 dotfiles 跟着 git 同步。

## 命令

| 命令 | 作用 |
| --- | --- |
| `mdp` | 打开交互选择器（方向键选，回车应用） |
| `mdp store <名字> [描述]` | 存下当前布局（同名需 `--force`） |
| `mdp list` | 列出方案，`*` 表示当前正在使用 |
| `mdp apply <名字>` | 应用方案（别名 `use`） |
| `mdp apply <名字> --dry-run` | 只看会怎么动，不真改 |
| `mdp apply <名字> --session` | 只对本次登录生效，不写永久配置 |
| `mdp current` | 显示当前布局 |
| `mdp show <名字>` | 方案详情 |
| `mdp rename <旧> <新>` | 重命名 |
| `mdp remove <名字> --yes` | 删除 |
| `mdp edit` | 用 `$EDITOR` 打开方案文件 |
| `mdp path` | 打印方案文件路径 |

全局选项：`--config <路径>`、`--json`。

### 选择器按键

| 按键 | 作用 |
| --- | --- |
| `↑` `↓` `←` `→` / `hjkl` | 移动选中项 |
| `1`–`9` | 选中第 N 项并直接应用 |
| `PageUp` `PageDown` / `Home` `End` | 翻页 / 跳首尾 |
| `Enter` | 应用选中方案 |
| `r` | 重新读取方案文件（外部改过之后用） |
| `q` / `Esc` / `Ctrl-C` | 取消，不动显示器 |

### 脚本里用

```bash
$ mdp apply work --json | jq -r '.displays[] | "\(.display) → \(.actual[0]),\(.actual[1])"'
External Display · 1920x1080 → 0,0
Built-in Display · 3024x1964 → 1920,268
```

`--dry-run` 的 JSON 用 `from` / `to` / `match` 描述「打算怎么动」，`match` 是 `uuid` 还是回退匹配一眼可见：

```bash
$ mdp apply work --dry-run --json | jq -r '.entries[] | "\(.display): (\(.from[0]),\(.from[1])) \(.match) → (\(.to[0]),\(.to[1]))"'
External Display · 1920x1080: (0,0) uuid → (0,0)
Built-in Display · 3024x1964: (1920,268) uuid → (1920,268)
```

## 配置文件

默认 `~/.config/mdp/solutions.toml`，可用 `--config` 或 `$MDP_CONFIG` 覆盖：

```toml
[[solution]]
name = "work"
description = "外接为主屏，笔记本放右边"
created_at = "2026-10-09T20:21:28+08:00"
updated_at = "2026-10-09T20:21:28+08:00"

[[solution.displays]]
uuid = "F425DECB-68B1-4C44-A90E-E38EEE5257E4"
name = "External Display"
x = 0            # 全局坐标，原点 = 主屏左上角
y = 0
width = 1920     # 逻辑尺寸（点）
height = 1080
pixel_width = 1920
pixel_height = 1080
builtin = false
primary = true   # ★ 这块是主屏

[[solution.displays]]
uuid = "37D8832A-2D66-02CA-B9F7-8F30A301B230"
name = "Built-in Display"
x = 1920
y = 268
width = 1512
height = 982
pixel_width = 3024
pixel_height = 1964
builtin = true
primary = false
```

用 `[[solution]]` 数组而不是 map，是为了让**文件里的顺序 = `mdp list` 的顺序**，手动调整顺序直接改文件。写入是原子的（先写临时文件再 `rename`），中途断电不会写坏配置。

> 删掉 `description` / `updated_at` 这些可选字段不影响使用。

## 工作原理

**显示器身份**用 `CGDisplayCreateUUIDFromDisplayID` 拿到的持久 UUID 作主键。`CGDirectDisplayID`（那个 `1` / `3` 之类的小整数）每次开机、每次插拔都会变，绝对不能当 key。

**布局切换**是一次事务：

```
CGBeginDisplayConfiguration
  → CGConfigureDisplayOrigin(主屏, 0, 0)      ← 原点在 (0,0) 的那块屏就是主屏
  → CGConfigureDisplayOrigin(其他屏, x, y)
CGCompleteDisplayConfiguration(kCGConfigurePermanently)
```

- 主屏必须**第一个**下发：全局坐标系的原点跟着主屏走，先改主屏再摆其他屏才不会整体偏移
- 中途任何一条配置失败 → `CGCancelDisplayConfiguration` 回滚
- 用 `kCGConfigurePermanently`，布局重启 / 重新登录后仍生效
- 提交后 sleep 120ms 回读真实位置 —— 窗口服务器是异步落地的，不能假设调用成功就等于生效

**只存位置和主屏，不存分辨率 / 刷新率。** 切分辨率的风险远大于收益，真要切用系统设置更合适。存的 `pixel_width` / `pixel_height` 只用于 UUID 匹配不上时的回退识别。

## 已知限制

- **macOS 会自动消除空隙和重叠。** 存下来的布局本来就无空隙，但如果两块屏之间的期望位置放不下，系统会挪到最近的可行位置。应用后 mdp 会回读并明确告诉你哪块屏没落在期望点上，而不是假装成功。
- **镜像状态下不配置位置。** 系统设置里开了「镜像」的屏位置由主屏决定，mdp 会跳过并警告；给非主屏设置位置时系统会自动解除镜像（CoreGraphics 文档写明的行为）。
- **显示器名字拿不到产品名。** Apple Silicon 上 `CGDisplayCreateDisplayFromIOKitPort` 已从 CoreGraphics 移除、`IODisplayConnect` 类也不再注册，所以名字只能是 `Built-in Display` / `External Display`（多块时带序号），区分靠尺寸 + UUID。
- **UUID 匹配不上时的回退**：按「像素尺寸 + 内建 / 外接」匹配，**唯一候选才敢用**；有多个同尺寸候选就报歧义并跳过（宁可不动也不要错位），同时提示你重存一次方案。
- **休眠的屏不会被配置**，只跳过并警告。
- 需要 macOS 11+。

## 开发

```bash
cargo test                                              # 匹配逻辑 + TOML 往返
cargo clippy --all-targets -- -D warnings
cargo fmt --all
mise run check                                          # test + lint
```

代码结构：

| 文件 | 职责 |
| --- | --- |
| `src/cg.rs` | CoreGraphics / CoreFoundation FFI，只封装公开 API |
| `src/model.rs` | `Solution` / `Placement` 数据模型与校验 |
| `src/store.rs` | 方案文件读写（原子写、路径解析） |
| `src/apply.rs` | 方案 ↔ 显示器匹配、下发布局、回读校验 |
| `src/tui.rs` | 两栏选择器（crossterm） |
| `src/cli.rs` | clap 命令行定义 |

改动构建配置后跑 `cargo dist generate` 让 release workflow 与配置同步。发版流程见 [RELEASING.md](RELEASING.md)。

## 卸载

```bash
brew uninstall mdp        # 或 brew untap define9/mdp
```

方案文件不会被删除，它在 `~/.config/mdp/solutions.toml`，要删自己 `rm`（或者留着，换工具后还能用）。

## License

[MIT](LICENSE)

---

## English

**mdp** — store and switch macOS multi-display layouts (positions + primary display) from the CLI.

```bash
brew tap define9/mdp && brew install mdp

mdp store work "external as primary, laptop on the right"
mdp                      # picker: arrows to move, Enter to apply
```

- Stores per-display positions and which display is primary
- Uses only public CoreGraphics APIs — **no Accessibility / Screen Recording permission**
- Atomic layout transactions with rollback; reports when macOS nudges a display
- JSON output for scripting (`mdp apply work --json`)
- One text file at `~/.config/mdp/solutions.toml`, dotfiles-friendly

Built with Rust. MIT licensed.
