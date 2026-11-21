# Spec Delta

## Purpose

让同一个问答界面通过 OpenAI 兼容、Anthropic Messages、Gemini 原生三种主流协议与服务商通信，并把各协议的流式响应统一成单一增量事件流。

## ADDED Requirements

### Requirement: OpenAI 兼容协议

对协议为 OpenAI 兼容的服务商，系统 SHALL 向 `{接口地址}/chat/completions` 发送带 Bearer 鉴权的流式请求，请求体包含模型、消息数组与 `stream: true`，并从 `choices[0].delta.content` 提取增量。

#### Scenario: 流式回答

- **WHEN** 用户对 OpenAI 兼容服务商提问并收到 SSE 分片
- **THEN** 每个非空 `delta.content` 都作为一段增量推给界面，`data: [DONE]` 表示结束

#### Scenario: 分片整形与心跳

- **WHEN** 收到 `data:` 前缀行、空行、注释行或无法解析的 JSON
- **THEN** 空行与注释被忽略，无法解析的行不产生增量，也不中断整个回答

#### Scenario: 国内兼容端点

- **WHEN** 服务商地址是兼容模式地址（如以 `/v1`、`/api/paas/v4` 结尾）且模型名为该厂商模型
- **THEN** 请求路径为在该地址后拼接 `/chat/completions`，鉴权与解析行为与 OpenAI 一致

### Requirement: Anthropic Messages 协议

对协议为 Anthropic 的服务商，系统 SHALL 向 `{接口地址}/messages` 发送流式请求，鉴权使用 `x-api-key` 与 `anthropic-version` 头，系统提示放入顶层 `system` 字段，并 MUST 携带 `max_tokens`。

#### Scenario: 流式回答

- **WHEN** 收到 `content_block_delta` 事件且其 `delta.text` 非空
- **THEN** 该文本作为增量推给界面，`message_stop` 表示结束

#### Scenario: 系统提示的位置

- **WHEN** 请求包含系统提示与多轮历史
- **THEN** 系统提示作为顶层 `system` 字段发送，`messages` 中只包含 user/assistant 轮次

#### Scenario: 错误事件

- **WHEN** 流中出现 `error` 事件
- **THEN** 系统把其中的错误信息作为错误结果推给界面并结束该次回答

### Requirement: Gemini 原生协议

对协议为 Gemini 的服务商，系统 SHALL 向 `{接口地址}/models/{模型}:streamGenerateContent` 发送流式请求并带 `alt=sse`，鉴权使用 `x-goog-api-key` 头，消息体为 `contents[].parts[].text`，系统提示放入 `systemInstruction`。

#### Scenario: 流式回答

- **WHEN** 收到含 `candidates[0].content.parts[].text` 的分片
- **THEN** 该文本作为增量推给界面，流结束或收到结束标记时结束该次回答

#### Scenario: 角色映射

- **WHEN** 历史中包含 assistant 轮次
- **THEN** 该轮次在请求体中以 `role: "model"` 发送，用户轮次以 `role: "user"` 发送

#### Scenario: 相邻同角色消息

- **WHEN** 转换后的 contents 出现两个相邻的同角色消息
- **THEN** 二者合并为一条，以满足交替要求

### Requirement: 统一的流事件与失败呈现

三种协议 SHALL 把结果统一为"增量 / 结束 / 错误"三类事件；非 2xx 响应 SHALL 以错误事件返回并携带状态码与服务商响应片段。

#### Scenario: 非 2xx 响应

- **WHEN** 服务商返回 4xx 或 5xx
- **THEN** 界面收到错误事件，内容包含状态码与服务商响应体片段，且该次回答结束

#### Scenario: 缺少 Key

- **WHEN** active 档案没有任何可用 Key
- **THEN** 界面收到提示 Key 缺失的错误并引导进入设置页，不发出网络请求

#### Scenario: 请求进行中的重复提交

- **WHEN** 上一次回答仍在流式输出时用户再次提交
- **THEN** 界面不发出第二个请求，直到当前回答结束或失败