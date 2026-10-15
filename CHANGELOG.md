# 更新日志

LovelyLauncher 的版本变化都记在这里。版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## v0.2.0 · 2026-10-15

### ✨ 新功能

**🔄 自动更新**

- 启动后在后台检查 [GitHub Releases](https://github.com/LovelyTeams/LovelyLauncher/releases) 是否有新版本，每天最多一次，可在设置中关闭
- 有新版本时，设置按钮上会出现小红点；在 **设置 → 通用 → 关于与更新** 里能看到更新说明，点“下载并更新”即可
- 下载后用 `SHA256SUMS.txt` 校验，原地替换并自动重启，配置和数据都不受影响

**👀 空格快速预览**

- 在资源管理器或桌面选中文件按 **空格** 就能预览，**↑ ↓ ← →** 切换文件，再按空格或 **Esc** 关闭
- 预览窗口不会抢焦点；重命名、地址栏、搜索框里打字时空格照常输入
- 搜索结果里的文件也可以右键“快速预览”，在 **设置 → 预览** 中可关闭此功能
- 支持的格式：
  - **Markdown**：完整渲染，包括标题、列表、任务列表、表格、代码块和本地图片
  - **图片**：png / jpg / gif（动画）/ bmp / webp / ico / tiff / svg
  - **文本与代码**：自动识别 UTF-8 / UTF-16 / GBK 编码
  - **表格**：`.xlsx`（多工作表）、`.csv`、`.tsv`
  - **Word 文档**：`.docx`，包括标题、加粗 / 斜体、列表、表格和内嵌图片
  - **音频**：mp3 / wav / wma / mid 无需安装任何东西；flac / ogg / m4a 等需要 FFmpeg
  - **视频**：mp4 / mkv / mov / webm 等，需要 FFmpeg
  - **文件夹**：列出内容与大小
- **FFmpeg** 可以一键下载（约 110 MB），也可以选择本机已有的 `ffmpeg.exe`；`PATH` 里有的话会自动使用

**🎨 代码高亮与美化**

- 支持 20 多种语言的语法高亮：Rust、JavaScript / TypeScript、Python、C / C++、Java / Kotlin、C#、Go、PHP、Shell、PowerShell、批处理、SQL、JSON、CSS、HTML / XML、YAML、TOML / INI、diff 等
- 压缩成一行的 **JSON**（包括带注释的 JSONC）和 **XML** 会自动排版，键的顺序、注释、数字写法都原样保留；标题栏可以在“格式化”和“查看原文”之间切换
- Markdown 里的代码块和 `@ai` 回答里的代码块也会按语言高亮

**📜 长行与滚动条**

- 文本默认**自动换行**；关掉后改成左右滚动，行号固定在左侧，选择会被记住
- 文本、表格、Markdown、Word 和文件夹预览都有可拖动的滚动条，也可以点击轨道直接跳转
- 表格可以左右滚动，行号列固定不动；`Shift + 滚轮` 或触控板横向滑动可以左右滚

### 💄 改进

- `@ai` 的回答改用完整的 Markdown 渲染，支持表格、引用、任务列表和代码块复制
- 更新说明在设置里以 Markdown 显示

### 📦 其他

- 项目采用 [AGPL-3.0](https://github.com/LovelyTeams/LovelyLauncher/blob/main/LICENSE) 许可证

## v0.1.0 · 2026-10-14

第一个公开版本。

- 全局快捷键呼出，支持模糊搜索和拼音首字母
- 收藏、最近使用的工具和项目、快捷操作
- “全部工具”方块图标网格，支持自定义分类和拖拽归类
- 每个工具都可以换自定义图标，默认使用程序自带的图标
- `Alt+Space` 迷你搜索框，可一键还原完整窗口
- 接入 Everything 搜索文件
- `@ai` 快速提问，内置 OpenAI、DeepSeek、Claude、Kimi、智谱 GLM，可自动拉取模型列表
- 网页搜索引擎可在设置中更换，默认 Bing
- 主题色、背景色调、毛玻璃和不透明度都能自定义
- 预置 LovelyMem、LovelySpark 一键下载
- 托盘常驻、开机自启、单实例，适配高分屏和高缩放
