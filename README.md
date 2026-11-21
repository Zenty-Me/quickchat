# quickchat

按一下热键就能问的桌面小工具。Windows 单文件 exe（约 4 MB），Tauri v2 + WebView2，无前端打包器。

按 `Ctrl+Shift+Space` 弹出悬浮窗，剪贴板里的文本自动带入，回车发送，答案是流式的，支持 Markdown。按 Esc 或点到别处就消失，不打断当前工作。

## 特性

- **全局热键**：`Ctrl+Shift+Space` 唤出/隐藏，托盘常驻兜底
- **剪贴板直觉**：唤出时剪贴板有文本（≤4000 字）就自动粘贴成问题
- **流式 Markdown**：边收边渲染，代码块、列表、引用、粗体都支持
- **多服务商**：可保存多份配置随时切换，支持三种协议

| 协议 | 接口地址填到 | 认证 |
| --- | --- | --- |
| OpenAI 兼容 | `https://api.openai.com/v1` | `Authorization: Bearer` |
| Anthropic | `https://api.anthropic.com/v1` | `x-api-key` + `anthropic-version` |
| Gemini | `https://generativelanguage.googleapis.com/v1beta` | `x-goog-api-key` |

OpenAI 兼容这一栏能覆盖通义千问、DeepSeek、Kimi、智谱、SiliconFlow、OpenRouter，以及本机的 Ollama（`http://127.0.0.1:11434/v1`）。

- **Key 不进前端**：请求全部由 Rust 侧发出，Key 存在 Windows 凭据管理器
- **测试连接 / 拉取模型**：填完地址和 Key 先验一遍，再把模型列表拉下来挑
- **单文件绿色版**：`bundle.active = false`，产出就是一个 exe，不写注册表

## 快速开始

系统要求：Windows 10/11（需要 WebView2 运行时，Win11 自带）、Rust 1.77+ 和 MSVC 工具链。

```powershell
git clone https://github.com/Zenty-Me/quickchat.git
cd quickchat\src-tauri
cargo build --release
# 产物：src-tauri\target\release\quickchat.exe
```

首次运行窗口会停在设置页，填服务商、地址、模型、Key 后保存即可。也可以直接双击 exe 后按热键唤出。

## 使用

- `Ctrl+Shift+Space` 唤出/隐藏，Esc 隐藏，失焦自动隐藏
- Enter 发送，Shift+Enter 换行
- 答案下方：复制 / 继续追问（带上一条上下文）/ 新问题
- 托盘菜单：显示、设置、开机自启、退出

对话只存在内存里，关掉窗口就没了；`继续追问` 才会带上上一轮。

## 服务商设置

设置页顶部的下拉切换服务商，可保存多个；`新建服务商` 里有常见厂商预设，末尾是自定义。每份配置包含名称、协议、接口地址、模型、API Key。

- **接口地址填到版本号为止**，不要带 `/chat/completions`、`/messages` 这类端点路径
- **拉取模型**：按当前表单取的地址和 Key 拉一次列表，点模型框即可挑选；取不到就手填
- **测试连接**：验证当前表单（含还没保存的改动），先试模型列表，失败再试一次 1 token 的对话，按 401/403（Key 无效）、404（地址不对）、模型不存在、连不上分别给提示
- **预设里的模型名会过时**，直接改模型框即可

Anthropic 协议会带上 `max_tokens: 2048`（该协议必填），所以超长回答会在 2048 token 处截断。

## 配置与凭据

配置文件在 `%APPDATA%\com.quickchat.app\config.json`：

```json
{
  "hotkey": "Ctrl+Shift+Space",
  "active": "p1",
  "providers": [
    { "id": "p1", "name": "OpenAI", "protocol": "openai",
      "base_url": "https://api.openai.com/v1", "model": "gpt-4o-mini" }
  ]
}
```

- API Key 按服务商分开存在 Windows 凭据管理器（条目名 `<id>.quickchat`），不落配置文件；删除服务商会连带删除它的 Key
- 旧版单服务商配置（只有 `base_url` / `model`）首次启动自动迁移：原文件备份为 `config.json.bak`，旧 Key 挪到新档案下。回滚就用 `.bak` 覆盖 `config.json`
- 环境变量 `QUICKCHAT_API_KEY` 可作为任意服务商的 Key 兜底（调试用，优先级低于凭据管理器）

## 结构

```
ui/                         静态前端，无打包器
  index.html                问答页 + 设置页两个视图
  app.js                    状态机、流式渲染、手写 Markdown
  style.css                 深色主题
src-tauri/src/main.rs       热键、托盘、配置与凭据、Tauri 命令、流式转发
src-tauri/src/protocol.rs   三种协议的端点/请求体/认证头/增量解析（含单测）
openspec/                   变更记录（add-provider-profiles）
```

## 已知限制

- 仅 Windows；exe 未签名，SmartScreen 可能拦截，选“仍要运行”
- Markdown 是手写的子集，不支持表格和嵌套列表
- 模型列表只取第一页（多数服务商的 `/models` 就一页）
- 长时间回答受 Anthropic 的 `max_tokens: 2048` 限制

## 许可

MIT