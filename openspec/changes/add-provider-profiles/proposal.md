# Proposal

## Why

设置里只有一个 OpenAI 兼容槽位，key 也只有一把：想在国内厂商、OpenRouter、本地模型之间切换，就得反复改地址、覆盖 key；想直接用 Anthropic 或 Gemini 的官方 key，协议层根本发不出去。而配置对不对（地址错、key 失效、模型名不存在）现在只能靠"发一条消息看看"才知道，用户拿到一句 HTTP 400 不知道改哪个字段。

## What Changes

- 配置从单一对象改为**服务商档案列表 + active 指针**；每个档案有 id、名称、协议、接口地址、模型
- key 按服务商分别存 Windows 凭据管理器（`user = provider.id`），切换服务商不再互相覆盖
- 新增 **Anthropic Messages** 与 **Gemini 原生** 两种协议适配，OpenAI 兼容保持不变（覆盖国内厂商全家桶与各种兼容网关）
- 设置页新增：服务商下拉 + 新建/删除、协议选择、**拉取模型列表**（可搜索可手填）、**测试连接**（先探模型列表，再回落一次 1-token 对话，错误按 401/404/模型不存在/不可达归因）
- 设置页内容超出固定窗口高度时**可滚动**；切换服务商时自动保存地址/模型/协议，key 单独提交
- 首次读到旧的单服务商配置与旧 key 时自动迁移到档案结构
- **BREAKING**：`config.json` 结构变更（`providers[]` + `active`）。只做一次读入迁移，不做长期向后兼容

## Capabilities

### New Capabilities

- `provider-config`: 服务商档案的增删改查与 active 选择、按服务商存储和校验 key、模型列表拉取、连接测试与错误归因、设置页的档案编辑交互
- `protocol-adapters`: OpenAI 兼容 / Anthropic / Gemini 三种协议的端点拼接、鉴权头、请求体构造、流式事件解析与角色映射

### Modified Capabilities

无。项目此前没有 spec（`openspec list --specs` 为空），现有对话与热键行为以代码为准，本次不改其需求。

## Impact

- `src-tauri/src/main.rs`：`Config` 结构改为档案列表；`key_status`/`set_key`/`ask` 增加 provider 维度；新增 `list_models`、`test_connection` 命令；旧配置迁移
- 新增 `src-tauri/src/protocol.rs`：协议枚举、请求构造、流片段解析及单测（现 `extract_delta` 移入）
- `ui/index.html`、`ui/app.js`、`ui/style.css`：设置页重排（服务商管理、协议选择、模型列表、测试连接、滚动）
- `README.md`：配置与协议说明更新
- 依赖不变：`reqwest`、`serde`、`keyring` 已满足三种协议与凭据需求
- 数据影响：`%APPDATA%\com.quickchat.app\config.json` 结构变更并在首次读取时迁移；旧 keyring 条目 `quickchat/api-key` 迁移后删除