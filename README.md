<div align="center">

<img src="assets/icon.png" width="128" alt="LovelyLauncher" />

# LovelyLauncher

**小工具，大进步。** 一个轻量、快捷的 Windows 工具启动器。

<p>
  <img alt="Windows 10/11" src="https://img.shields.io/badge/Windows-10%20%7C%2011-ee6585?style=flat-square&logo=windows11&logoColor=white" />
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2024-ee6585?style=flat-square&logo=rust&logoColor=white" />
  <img alt="GPUI" src="https://img.shields.io/badge/UI-GPUI-ee6585?style=flat-square" />
</p>

[功能亮点](#-功能亮点) · [截图](#-截图) · [快速开始](#-快速开始) · [使用指南](#-使用指南) · [AI 助手](#-ai-助手) · [配置](#%EF%B8%8F-配置文件) · [性能](#-性能) · [更新日志](CHANGELOG.md)

<br />

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/dashboard-dark.png" />
  <img src="docs/images/dashboard-light.png" width="860" alt="LovelyLauncher 主界面" />
</picture>

</div>

---

## ✨ 功能亮点

<table>
<tr>
<td width="50%" valign="top">

**⚡ 轻、快**
- 基于 [GPUI](https://gpui.rs)
- 单文件 exe 约 7 MB，upx压缩后不到 3M，常驻内存约 30 MB，**隐藏到托盘后不到 1 MB**

**🔍 一个输入框搞定一切**
- 工具、开始菜单应用、项目、快捷操作统一模糊搜索
- **拼音**全拼和首字母：`rwgl` → 任务管理器，`ji` → 记事本 / 计算器
- 接入 [Everything](https://www.voidtools.com/)，顺便搜文件
- `>命令` 直接执行，输入网址直接打开，其余一键网页搜索

</td>
<td width="50%" valign="top">

**🤖 `@ai` 随手问**
- `@ai 问题` 回车即问，答案流式显示，代码块一键复制
- 问过的问题自动存进**历史对话**，可筛选、可重新打开
- 预置 OpenAI、DeepSeek、Claude、Kimi、智谱 GLM，模型列表直接从接口拉取

**🎀 好看，也好改**
- 收藏卡片 + 图标格子 + **自定义分类**（拖图标到分类标签即可归类）
- 每个工具都能**上传自定义图标**，默认用程序自带图标
- 主题色、背景色调、毛玻璃、不透明度都能调，还能逐项覆盖颜色

**👀 空格快速预览**
- 在资源管理器或桌面选中文件按**空格**即可预览，方向键切换、再按空格关闭
- Markdown、图片、文本（自动识别 GBK）、PDF、网页、Word、Excel、CSV / TSV、压缩包、音频、视频
- 代码**语法高亮**，压缩过的 JSON / XML **自动美化**，一键切回原文

**🪟 用着顺手**
- `Alt+Space` 呼出**迷你搜索框**，一键还原完整窗口
- 单独**按住 Alt**，右下角弹出果冻质感的**圆环菜单**，指向图标松开 Alt 就能打开；选中文字时弹出**划词圆环**（翻译、解释、总结…）
- **剪贴板历史** + **文本片段**：`Alt+Shift+V` 随时找回复制过的内容，`Enter` 粘贴回原窗口
- **取色器**、**屏幕标尺**，一键量尺寸、取颜色
- 从 GitHub **自动更新**（SHA256 校验后原地替换）
- 托盘常驻、开机自启、单实例、高分屏 / 高缩放自适应

</td>
</tr>
</table>

## 📸 截图

<table>
<tr>
<td align="center" width="50%"><b>迷你搜索框</b>（<code>Alt+Space</code>，支持拼音）<br /><br /><img src="docs/images/mini-search.png" alt="迷你搜索框" /></td>
<td align="center" width="50%"><b><code>@ai</code> 快速提问</b><br /><br /><img src="docs/images/ai-mini.png" alt="AI 快速提问" /></td>
</tr>
<tr>
<td align="center"><b>自定义主题</b><br /><br /><img src="docs/images/settings-theme.png" alt="主题设置" /></td>
<td align="center"><b>AI 服务商设置</b><br /><br /><img src="docs/images/settings-ai.png" alt="AI 设置" /></td>
</tr>
</table>

<details>
<summary>深色主题</summary>
<br />
<img src="docs/images/dashboard-dark.png" width="860" alt="深色主题" />
</details>

> 截图使用的是演示数据；`@ai` 截图里的回答来自本地模拟接口。

## 🚀 快速开始

### 构建

需要 [Rust](https://rustup.rs/)（MSVC 工具链）和 Visual Studio 的 **C++ 生成工具**。

```bash
cargo build --release
```

产物是单个文件 `target/release/LovelyLauncher.exe`，复制到任意位置即可运行。

### 发布新版本

仓库自带 GitHub Actions 工作流（[`.github/workflows/release.yml`](.github/workflows/release.yml)）：

1. 修改 `Cargo.toml` 里的 `version` 并提交；
2. 打标签并推送：`git tag v0.2.0 && git push origin v0.2.0`。

工作流会依次完成：检查标签和版本号是否一致 → 运行测试 → 编译 release 版 → **用 UPX 压缩**（`--best --lzma`）并校验 → 冒烟测试 → 打包 exe、zip 和 SHA256 校验文件 → 自动创建 GitHub Release，并生成更新说明。带连字符的标签（如 `v0.2.0-beta.1`）会发布为预发布版本。也可以在 Actions 页面手动运行，此时只编译并上传构建产物，不会创建 Release。

### 第一次使用

1. 运行后会出现主界面，同时在托盘里常驻。
2. 按 **`Alt+Space`** 随时呼出搜索框，再按一次隐藏。
3. 把常用的程序、脚本、文件夹**直接拖进窗口**，就成了工具（正在看哪个分类标签，就加到哪个分类）。
4. 想用 `@ai`：打开 **设置 → AI 助手**，填写 API Key，点“拉取模型”选一个。

> 如果 `Alt+Space` 被 PowerToys Run 等程序占用，可以在 **设置 → 通用** 里换成别的组合键。

## 📖 使用指南

### 搜索框能做什么

| 输入 | 效果 |
|---|---|
| `任务` / `rwgl` / `renwu` | 模糊搜索工具、应用、项目和快捷操作，支持拼音全拼和首字母 |
| `notes.md` | 同时显示 Everything 找到的文件和文件夹（需要 Everything 在运行） |
| `>ipconfig /all` | 在新的命令行窗口中执行命令 |
| `github.com` | 直接在浏览器打开网址 |
| `@ai 怎么查看端口占用` | 按 Enter 向默认 AI 服务商提问 |
| `@claude …` / `@deepseek …` | 指定服务商提问（`openai`、`deepseek`、`claude`、`kimi`、`glm`） |
| `cb` / `cb 关键词` | 剪贴板历史和文本片段，`Enter` 粘贴回原来的窗口 |
| 其它任何内容 | 列表最后一项可以用网页搜索（默认必应，可在设置里更换） |

### 快捷键

| 按键 | 作用 |
|---|---|
| `Alt+Space` | 呼出 / 隐藏（默认弹出迷你搜索框） |
| `↑` `↓` · `Enter` | 选择 · 打开搜索结果；`@ai` 模式下为提问，历史对话里为打开 |
| `Ctrl+Enter` | 以管理员身份运行 |
| `Esc` | 返回（历史对话）→ 停止 AI 回答 → 清空搜索 → 隐藏窗口 |
| `Ctrl+K` | 聚焦搜索框 |
| `Ctrl+T` · `Ctrl+O` | 新建终端 · 打开项目 |
| `Ctrl+Shift+S` · `Ctrl+N` | 截图 · 快速笔记 |
| `Ctrl+Shift+Q` | 快速扫码：识别屏幕上和剪贴板图片里的二维码、条形码 |
| `Ctrl+Shift+C` · `Ctrl+Shift+R` | 取色器 · 屏幕标尺 |
| `Alt+Shift+V`（全局）· `Ctrl+Shift+V` | 剪贴板历史 |
| `Ctrl+D` · `Ctrl+,` | 添加工具 · 打开设置 |

### 工具、分类与图标

- **添加工具**：拖文件进窗口，或点“添加工具”，填写程序路径、网址，或者 `ms-settings:` 这类协议地址都可以。也可以直接粘贴整行命令（比如快捷方式属性里的“目标”），会自动拆成程序和参数；快捷方式的“起始位置”对应“工作目录”，留空时程序从自己所在的文件夹启动。拖进来的 `.lnk` 快捷方式会原样保留它的参数和起始位置。
- **导入开始菜单**：在“添加工具”对话框点“导入开始菜单程序”，可以把开始菜单里“已固定”的程序一次性导进来（按固定顺序，支持拼音搜索、全选 / 反选和选择分类；需要 Windows 11）。
- **排序**：在“全部工具”里**把图标拖到另一个图标上**，就会换到那个位置；拖到“添加工具”上则移到最后。分类标签也可以拖动排序。
- **收藏**：把图标拖到“收藏工具”区域即可收藏，收藏之间也能拖动排序（和“全部工具”的顺序互不影响）。收藏多于一行时改为小图标，最多两行，其余滚动查看。
- **分类**：点“＋ 分类”新建；**把图标拖到分类标签上**即可归类；右键分类可以重命名或删除。
- **图标**：编辑工具时点“上传图标…”，支持 PNG / JPG / ICO，也可以选一个 exe 提取它的图标；不上传就用程序自带的图标。
- **右键菜单**：打开、以管理员身份运行、收藏、编辑、打开文件位置、删除。
- **预设下载**：LovelyMem、LovelySpark 第一次点开时会自动下载（卡片上显示进度），下载完自动启动；右键可以重新下载来更新。

### 空格快速预览

LovelyLauncher 运行时，在**资源管理器或桌面**选中一个文件，按 **空格** 就会弹出预览窗口：

- 预览窗口不会抢走焦点：继续按 **↑ ↓ ← →** 切换文件，预览会跟着变；再按 **空格** 或 **Esc** 关闭
- 文字可以**拖动选中**后按 `Ctrl+C`（或点标题栏的“复制”）复制，双击选词、三击选整段，`Ctrl+A` 全选；文本 / 代码、Markdown、网页、Word 都支持
- 拖动右下角的手柄（或窗口边缘）调整窗口大小，标题栏的按钮可以最大化；调整后的大小会被记住
- 只在文件列表里生效，重命名、地址栏、搜索框里打字时空格照常输入
- 搜索结果里的文件也可以右键“快速预览”
- 可在 **设置 → 预览** 中关闭

| 类型 | 格式 | 说明 |
|---|---|---|
| Markdown | `.md` | 完整渲染：标题、列表、任务列表、表格、代码块、本地图片 |
| 图片 | png / jpg / gif / bmp / webp / ico / tiff / svg | GIF 支持动画 |
| 文本与代码 | txt / log / json / xml / yaml / 各类源码…… | 语法高亮；JSON / XML 自动美化；自动识别 UTF-8 / UTF-16 / GBK，大文件只读前 4 MB |
| 表格 | `.csv` `.tsv` `.xlsx` | 行列号、多工作表切换，最多 10 万行 |
| 文档 | `.docx` | 标题、加粗 / 斜体、列表、表格、内嵌图片 |
| 音频 | mp3 / wav / wma / mid | Windows 自带，**无需额外安装** |
| 音频（更多） | flac / ogg / opus / m4a / aac…… | 需要 FFmpeg |
| 视频 | mp4 / mkv / mov / avi / webm…… | 需要 FFmpeg |
| PDF | `.pdf` | 用 Windows 自带的 PDF 引擎逐页渲染，滚到哪页渲染哪页 |
| 网页 | `.html` `.htm` | 阅读视图：标题、段落、列表、表格、代码、图片（含 base64 内嵌图片）；可切换到高亮源码 |
| 压缩包 | zip / 7z / rar / tar.gz / tar.xz / iso / cab / jar / apk…… | 按文件夹树列出内容、大小和时间，点文件夹可折叠；中文文件名（含 GBK 编码的 zip）正常显示 |
| 文件夹 | — | 列出内容与大小 |

**代码高亮与美化**

- 支持 Rust、JavaScript / TypeScript、Python、C / C++、Java / Kotlin、C#、Go、Swift、PHP、Ruby、Lua、Shell、PowerShell、批处理、SQL、JSON、CSS、HTML / XML、YAML、TOML / INI、diff 等，没有扩展名的脚本按 `#!` 行识别
- 压缩成一行的 JSON（包括带注释的 JSONC）和 XML 会自动缩进排版，键的顺序、注释、数字写法都原样保留；标题栏的“查看原文 / 格式化”可以来回切换
- Markdown 和 AI 回答里的代码块也会按语言高亮
- 超过 2 MB 的文件只显示纯文本，不做高亮

**长行与滚动**

- 文本默认**自动换行**；点标题栏的“自动换行”可以关掉，改成左右滚动（行号固定在左侧），选择会被记住
- 文本、表格、Markdown、Word 预览都有可拖动的滚动条；表格可以左右拖动，行号列固定不动
- 鼠标滚轮上下滚，`Shift + 滚轮` 或触控板横向滑动左右滚

**FFmpeg**：第一次预览视频时会提示，可以一键下载（来自 gyan.dev 的 essentials 版本，约 110 MB），也可以选择本机已有的 `ffmpeg.exe`；`PATH` 里有 ffmpeg 也会自动使用。

### 自动更新

启动后会在后台检查 [GitHub Releases](https://github.com/LovelyTeams/LovelyLauncher/releases) 是否有新版本（每天最多一次，可在设置中关闭）。发现新版本时设置按钮上会出现小红点，在 **设置 → 通用 → 关于与更新** 里点“下载并更新”：下载 → 用 `SHA256SUMS.txt` 校验 → 原地替换 → 自动重启。

### Alt 圆环菜单

单独按住 **Alt** 约 0.3 秒，屏幕右下角会弹出一个透明的圆环，上面是你的常用工具：

- 在“全部工具”里点 **圆环** 标签，把图标拖到这个标签上（或右键 → 加入圆环）就能自己挑，最多 8 个；不挑的话，圆环显示打开次数最多的 8 个

- 把鼠标移到某个图标上，**松开 Alt** 就会打开它；也可以直接点击图标
- 鼠标移回圆环中间再松开 Alt，什么都不打开
- 圆环弹出时会回弹、像果冻一样晃动，选中的高亮会像黏液一样拉长再收回，图标也会被鼠标轻轻“吸”过去；**弹性**可在设置里选关闭 / 轻柔 / 标准 / 夸张
- 按住 Alt 时按了别的键（比如 `Alt+Tab`、`Alt+F4`）不会弹出圆环，原有快捷键不受影响
- 前台是**全屏游戏或全屏视频**时（包括无边框全屏）不会弹出，打游戏长按 Alt 不会误触；普通最大化的窗口不受影响
- **排除的程序**：在 **设置 → 通用** 里添加（可以直接点正在运行的程序），这些程序在前台时不弹出圆环，适合窗口模式运行的游戏
- 打开 **圆环跟随鼠标位置** 后，圆环以鼠标为中心弹出（靠近屏幕边缘时会往里挪），不用再把鼠标移到右下角
- **快捷操作也能放进圆环**：在“快捷操作”里右键 → 加入圆环，排在工具后面
- 以上都可以在 **设置 → 通用** 里调整或关闭

**划词圆环**：先选中一段文字再按住 Alt，弹出的是蓝紫色的文字圆环，在鼠标旁边出现，中间引用选中的文字：

- **翻译 / 解释 / 总结**：打开 `@ai` 面板直接回答（搜索框里显示简短的问题，完整的文字一起发给 AI）
- **搜索**、**复制**、**存为片段**；选中的是网址或文件路径时还有 **打开**
- 文字通过 Windows 的无障碍接口（UI Automation）和编辑框直接读取，不碰剪贴板、不模拟按键；个别读不到的程序，先 `Ctrl+C` 复制再在 4 秒内按住 Alt 也会弹出文字圆环
- 没有选中文字时，按住 Alt 还是原来的工具圆环

### 剪贴板历史与文本片段

- 复制过的文字、图片和文件会记在本机（最多 200 条、图片 40 张，固定的另算）；密码管理器标记为不记录的内容会跳过
- 在任意窗口按 `Alt+Shift+V`（**设置 → 剪贴板** 里可以改），或在搜索框输入 `cb`，`cb 关键词` 筛选
- `Enter` 粘贴回刚才的窗口，`Ctrl+Enter` 只复制；右键可以固定、存为文本片段、删除
- **文本片段**：常用的签名、地址、模板。在 **设置 → 剪贴板** 里新建（内容可以多行），搜名称或在 `cb` 里都能找到；`{date}` `{time}` `{datetime}` `{clipboard}` 会在粘贴时替换成当前日期、时间和剪贴板文字

### 取色器与屏幕标尺

两个都在“快捷操作”里（也可以放进圆环），会先截下鼠标所在显示器的画面再盖在上面：

- **取色器**（`Ctrl+Shift+C`）：放大镜显示鼠标处的像素和颜色值，单击复制 HEX，`Shift` + 单击复制 `rgb()`，方向键逐像素移动
- **屏幕标尺**（`Ctrl+Shift+R`）：鼠标所在的同色区域会自动量出宽高；按住拖动可以量任意矩形，松开就复制“宽 × 高”（屏幕像素）
- `Esc` 或右键退出

### 快速扫码

在“快捷操作”里点 **快速扫码**（或按 `Ctrl+Shift+Q`），启动器会先让开，然后识别**所有显示器上**和**剪贴板图片里**的码：

- 支持二维码、Data Matrix、PDF417、Aztec，以及 EAN-13 / EAN-8 / UPC / Code 128 / Code 39 / ITF 等条形码，一次能识别多个
- 找到多个码时，结果上方会显示屏幕（或剪贴板图片）的缩略图，用带编号的彩色框标出每个码的位置，下面的结果用同样的编号和颜色，并附上码和周围内容的截图
- 只找到一个码时会直接复制内容；每条结果都可以复制，网址可以一键打开
- 二维码里的中文（包括 GBK 编码的）能正常显示

### 迷你搜索框

热键默认弹出一个只有输入框的小窗口，出现在鼠标所在显示器的偏上位置。打字后在下方展开结果，点“还原窗口”会回到完整界面，并恢复原来的位置和大小。点到别处时它会自动收起。不喜欢的话可以在 **设置 → 通用** 里关掉，热键就会直接打开完整窗口。

## 🤖 AI 助手

在 **设置 → AI 助手** 里为每家服务商填写接口地址、API Key 和模型。点“拉取模型”会请求服务商的 `/models` 接口，列出可用模型供你点选；也可以直接手动填写模型名。

| 服务商 | 默认接口地址 | 接口格式 |
|---|---|---|
| OpenAI | `https://api.openai.com/v1` | Chat Completions |
| DeepSeek | `https://api.deepseek.com/v1` | OpenAI 兼容 |
| Claude | `https://api.anthropic.com/v1` | Anthropic Messages（默认模型 `claude-opus-5-5`） |
| Kimi | `https://api.moonshot.cn/v1` | OpenAI 兼容 |
| 智谱 GLM | `https://open.bigmodel.cn/api/paas/v4` | OpenAI 兼容 |

- 回答以流式方式实时显示，支持标题、列表和代码块（代码块带语法高亮和“复制”按钮），`Esc` 可随时停止。
- **历史对话**：回答会自动保存（最多 100 条）。输入 `@ai` 后点提示下面的“历史对话”，或回答面板右上角的“历史”，就能看到以前的问答；在 `@ai` 后面打字可以筛选，`Enter` 打开，打开后再按 `Enter` 会重新提问。可以单条删除或清空，也可以在设置里关闭保存。
- 接口地址可以改成任何 OpenAI 兼容的服务，比如中转站或本地模型。
- ⚠️ API Key 以**明文**保存在本机的 `config.json` 里，历史对话保存在同一目录的 `ai_history.json` 里，请不要把这两个文件分享给别人。

## 🎨 主题

在 **设置 → 外观** 中可以调整：

- 主题模式：跟随系统 / 浅色 / 深色（标题栏的 ☀️ 按钮也能快速切换）
- 主题色：9 个预设，或输入任意 `#RRGGBB`，**实时生效**
- 背景跟随主题色、毛玻璃背景、背景不透明度（70% ~ 100%）
- 界面：分别显示或隐藏“收藏工具”“最近使用”“最近项目”“快捷操作”卡片

所有颜色都由主题色推导出来。想进一步微调的话，可以在 `config.json` 里按浅色 / 深色分别覆盖任意颜色项：

```jsonc
"theme_overrides": {
  "light": { "window_bg": "#FFF4F7", "accent": "#FF5C8A" },
  "dark":  { "panel": "#2A2030" }
}
```

<details>
<summary>可覆盖的颜色项</summary>

`window_bg` `panel` `panel_hover` `surface` `border` `border_strong` `text` `text_muted` `text_faint` `accent` `accent_soft` `accent_text` `input_bg` `selection` `kbd_bg` `kbd_border` `row_hover` `row_active` `overlay` `danger` `banner_bg` `banner_text` `folder`

</details>

### 壁纸

在 **设置 → 外观 → 壁纸** 里选择一张图片或一段视频作为窗口背景：

- 图片支持 png / jpg / webp / bmp，视频支持 mp4 / webm / mov / mkv / gif 等
- 可调 **填充方式**（填满 / 适应 / 拉伸）、**模糊**、**亮度**、**饱和度**、**遮罩浓度**，拖动滑块实时预览；遮罩越浓壁纸越淡、文字越清楚
- 视频静音循环播放，可调播放速度（0.5× ~ 2×），窗口隐藏时自动暂停，不占 CPU
- 视频壁纸需要 FFmpeg，没有的话设置里可以一键下载或选择本机的 `ffmpeg.exe`

## ⚙️ 配置文件

- 默认位置：`%APPDATA%\LovelyLauncher\config.json`（设置页左下角“打开配置文件夹”可以直接打开）
- 设置环境变量 **`LOVELYLAUNCHER_DATA`** 可以把数据放到别的目录，适合便携使用
- AI 历史对话保存在数据目录的 `ai_history.json`，剪贴板历史在 `clipboard.json`（图片在 `clipboard\`），文本片段在 `config.json`；自定义图标保存在数据目录的 `icons\` 下；预设程序下载到 `%LOCALAPPDATA%\LovelyLauncher\apps\`，FFmpeg 下载到 `%LOCALAPPDATA%\LovelyLauncher\tools\`
- 配置文件可以手动编辑（带不带 BOM 都行）；旧版本的配置会自动迁移

## 📊 性能

在 Windows 11、2560×1440 显示器上实测 release 版：

| 指标 | 数值 |
|---|---|
| 程序体积 | **7.4 MB**（单文件，无需运行时） |
| 窗口显示时内存（私有工作集） | **约 30 MB** |
| 隐藏到托盘后内存 | **< 1 MB**（隐藏 3 秒后把内存交还给系统） |

> “提交大小”约 100 MB，主要是显卡驱动和 Direct3D 的地址空间预留，并不占用物理内存。

## 🗂️ 项目结构

| 文件 | 职责 |
|---|---|
| `src/main.rs` | 启动、单实例、窗口创建、事件分发 |
| `src/launcher.rs` | 核心状态与逻辑：搜索、启动、下载、分类、迷你模式、AI 会话 |
| `src/view.rs` | 主界面渲染：仪表盘、图标格子、搜索结果、迷你框、AI 面板 |
| `src/dialogs.rs` | 编辑工具、设置（通用 / 外观 / 预览 / AI / 搜索）、分类对话框 |
| `src/preview/` | 空格预览：键盘钩子与选中文件读取（`hook.rs`）、预览窗口（`mod.rs`）、Office / CSV / 编码（`office.rs`）、PDF（`pdf.rs`）、网页（`html.rs`）、压缩包（`archive.rs`）、滚动条（`scrollbar.rs`）、播放器（`media.rs`）、FFmpeg（`ffmpeg.rs`） |
| `src/ring.rs` | 按住 Alt 弹出的圆环菜单（弹簧动画、黏液高亮），以及划词圆环 |
| `src/clipboard.rs` | 剪贴板历史（监听、保存、筛选）与文本片段变量 |
| `src/overlay.rs` | 取色器和屏幕标尺 |
| `src/wallpaper.rs` | 壁纸：图片处理与 FFmpeg 视频解码 |
| `src/scan.rs` | 快速扫码：截屏、读剪贴板图片、识别二维码和条形码 |
| `src/markdown.rs` | Markdown 渲染（预览与 AI 回答共用） |
| `src/select.rs` | 预览里的文字选择与复制 |
| `src/highlight.rs` | 语法高亮与 JSON / XML 美化 |
| `src/update.rs` | 从 GitHub Releases 自动更新 |
| `src/input.rs` | 支持输入法（IME）的单行输入框 |
| `src/search.rs` | 模糊匹配与拼音搜索 |
| `src/start_menu.rs` | 导入开始菜单“已固定”的程序 |
| `src/ai.rs` · `src/http.rs` | AI 接口（OpenAI 兼容 / Anthropic）与 WinHTTP 客户端 |
| `src/ai_history.rs` | AI 历史对话的保存、筛选和时间显示 |
| `src/everything.rs` | Everything IPC 文件搜索 |
| `src/win.rs` | Win32：窗口、图标提取、开始菜单扫描、开机自启 |
| `src/theme.rs` · `src/config.rs` | 主题生成、配置与迁移 |
| `build.rs` | 从 `assets/icon.png` 生成 exe 图标、托盘图标和应用内 Logo |

想换应用图标的话，替换 `assets/icon.png` 再重新编译即可。

## 📄 许可证

[AGPL-3.0](LICENSE)。各版本的变化见 [更新日志](CHANGELOG.md)。

## 🙏 致谢

- [GPUI](https://gpui.rs) — Zed 团队开源的 GPU 加速 UI 框架
- [Everything](https://www.voidtools.com/) — voidtools 的极速文件搜索
- 界面线条图标的路径参考了 [Lucide](https://lucide.dev)（ISC 许可）

<div align="center">
<br />
<sub>Made with 💗</sub>
</div>
