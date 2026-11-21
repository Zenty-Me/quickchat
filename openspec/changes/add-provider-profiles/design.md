# Design

## Context

现状（`src-tauri/src/main.rs`，362 行）：`Config { base_url, model, hotkey }` 单槽位；凭据固定 `keyring service="quickchat", user="api-key"`；`ask()` 写死 `POST {base_url}/chat/completions` + Bearer，`extract_delta()` 只认 `choices[0].delta.content`。前端设置页是单栏四个字段，`main` 无滚动，窗口固定 760x440 不可调整大小。

约束：单文件绿色 exe（`bundle.active=false`）；依赖不新增；窗口尺寸不可依赖放大；HTTP 客户端已用 `reqwest` + `futures-util`，凭据已用 `keyring`（windows-native）。

## Goals / Non-Goals

**Goals**

- 一个进程内并存多个服务商档案，active 决定 `ask()` 打向谁
- 三种协议共用同一套流事件（Delta/Done/Error），界面层感知不到协议差异
- 配置错误在设置页当场可诊断，不必用真实提问试错
- 旧配置文件与旧凭据在新版本首次启动时无损迁移

**Non-Goals**

- 不做 provider 插件化/动态加载，协议集合固定为三种
- 不做多 Key 轮询、额度统计、余额查询
- 不做系统代理、OAuth 登录流、OpenAI Responses API、工具调用、图片输入
- 不重排问答视图；模型与协议的可见性只体现在设置页

## Decisions

**1. 协议层独立成 `src-tauri/src/protocol.rs`，用枚举 + 两个纯函数分派，不上 trait**

```
enum Protocol { OpenAi, Anthropic, Gemini }

fn endpoint(p, base, model) -> String
fn build_body(p, model, messages) -> serde_json::Value
fn auth(p, req, key) -> RequestBuilder
fn parse_line(p, line) -> Delta      // Delta { Text(String), Done, Skip }
```

理由：变体固定、无运行期实现替换需求，trait 只能带来 `async fn` 或 `Box<dyn>` 的额外复杂度；纯函数可直接单测（现有 `parses_openai_sse` 平移过来再加两个）。否掉的替代方案：`trait ChatProtocol`（收益为零，成本是可见性/生命周期噪音）。

**2. 配置结构 `{ hotkey, active, providers[] }`，档案字段 `{ id, name, protocol, base_url, model }`**

`base_url` 沿用旧字段名，迁移时可直接映射。`id` 用毫秒时间戳生成（`format!("p{}", millis)`），改名不动 id，凭据跟着 id 走。

**3. 凭据 `user = provider.id`，`service` 保持 `"quickchat"`**

改 service 名会让已有凭据在新版本里"消失"；id 作为 user 天然实现"换服务商不覆盖"。删除档案时同步删条目，避免残留。

**4. 迁移在 `load_config` 内一次完成，并先备份**

读到不含 `providers` 的旧结构 → 构造单档案 → 把原文件复制为 `config.json.bak` → 写回新结构 → 把旧 `api-key` 凭据复制到新 id 后删除旧条目。任何一步失败都不阻断启动（读不到 key 只是未设置状态）。备份文件是唯一回滚路径。

**5. 三协议的请求差异集中在一处**

| | OpenAI 兼容 | Anthropic | Gemini |
|---|---|---|---|
| endpoint | `{base}/chat/completions` | `{base}/messages` | `{base}/models/{model}:streamGenerateContent?alt=sse` |
| 鉴权 | `Authorization: Bearer` | `x-api-key` + `anthropic-version: 2023-06-01` | `x-goog-api-key` |
| 系统提示 | `messages[0].role=system` | 顶层 `system` 字段 | `systemInstruction.parts[]` |
| 角色 | user/assistant | user/assistant | user/model |
| 必填补丁 | — | `max_tokens: 2048` | 相邻同角色合并 |
| 增量路径 | `choices[0].delta.content` | `content_block_delta.delta.text` | `candidates[0].content.parts[].text` |
| 结束标记 | `[DONE]` | `message_stop` | 流结束/`finishReason` |

Gemini 的角色映射与相邻合并放在 `build_body` 里做，因为它只影响请求构造，不影响事件解析。

**6. 模型列表用原生 `<input list>` + `<datalist>`**

自绘可搜索下拉要处理定位、键盘、层级，在一个不可调尺寸的 440 高窗口里不划算。datalist 同时满足"能搜索"和"能手填"，失败时只需清空 options。

**7. 测试连接复用请求构造，三级回退**

`test_connection` 在 Rust 侧直接调用 `list_models` 的请求；失败再发一次 `stream=false`、`max_tokens=1` 的对话（复用 `build_body` 但替换 `stream` 字段，Gemini 侧改 `generationConfig.maxOutputTokens`）。错误按状态码归类，原始响应片段截 300 字。测的是表单当前值而非已保存值，因此命令参数是"临时档案对象"而不是 provider id。

**8. 设置页单栏 + 滚动，服务商操作放顶部一行**

```
[服务商 v] [+ 新建] [删除]
名称 / 协议 / 接口地址 / 模型 + [拉取模型] / API Key
[测试连接]  状态文字
                        [保存] [返回]
```

否掉左右两栏：440 高下左栏即使只放 3 项也会把右栏表单推出可视区，且需要重写现有 `main` flex 结构。滚动只需给设置容器 `overflow-y: auto`。

**9. 切换服务商时自动保存非凭据字段**

切走前把当前表单的 name/protocol/base_url/model 写回原档案（不碰 key），再加载目标档案。理由：避免"未保存改动"提示与用户输入丢失；key 是显式动作，仍由保存按钮提交。

**10. 设置页的瞬时状态留在前端内存**

测试结果、模型候选存在 JS 模块级变量，视图切换（问答 ↔ 设置）不清空；重启后模型候选为空（见 spec）。

## Risks / Trade-offs

- [Gemini 不带 `alt=sse` 会返回 JSON 数组流] → 实现时先用 curl 打一发确认，单测覆盖解析器对两种形态不误判（解析器只认 `data:` 行，JSON 数组整体落在无 `data:` 前缀时会被忽略并报"无增量"）
- [各家 `/models` 路径与参数不一致：Anthropic `/v1/models`、Gemini `/v1beta/models`、部分兼容网关未实现] → 列表请求按协议分派，失败一律降级为手填 + 提示；测试连接的第二级回退保证"列表不可用但对话可用"仍能判绿
- [keyring 写入被系统策略拒绝] → `set_key` 返回错误原样显示，不静默成功
- [旧凭据迁移读取失败] → 保留旧条目、新档案显示未设置，不删数据
- [4xx 响应里"提到模型"是启发式判断，可能把别的错误报成模型问题] → 片段原文同时显示，用户可自行判断
- [SSE 按行切分，若上游把单条 `data:` 拆成多行会漏] → 现状已如此，三家主流实现都不这么发，不在本次修复
- [设置页滚动后右下角按钮随内容滚动] → 底部操作行放在滚动容器外，固定可见

## Migration Plan

1. 读取旧 `config.json`（无 `providers` 字段）→ 备份 `config.json.bak` → 写回新结构 → 迁移凭据
2. 迁移完成后旧格式不再被识别；回滚方式是把 `config.json.bak` 覆盖回去并删掉迁移产生的凭据条目
3. 无旧配置的新机：走默认档案，不产生备份文件

## Open Questions

- Anthropic/Gemini 模型列表的分页参数（`limit` / `pageSize`）以实现时的 curl 结果为准，不影响结构设计；若某家拒收参数就去掉参数重试一次
- 【结论，无真实 Key】本次没有 Anthropic / Gemini 可用 Key，1.4 的 curl 验证改用本地 mock 覆盖：OpenAI 形态 `data[].id`、Gemini 形态 `models[].name`（去 `models/` 前缀）、Anthropic 的 `data[].id` 均按官方文档形状构造并端到端跑通（Gemini 走 `/v1beta/models/{model}:streamGenerateContent?alt=sse`，Anthropic 走 `/v1/messages` 且 `system` 在顶层、`max_tokens` 必填，均已在真实请求体里确认）。据此决定：分页一律不传参，只取第一页；若账号模型数超过一页上限（Gemini 默认 50），列表会不全但手填仍可用