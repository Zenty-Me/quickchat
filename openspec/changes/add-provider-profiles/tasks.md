# Tasks

## 1. 协议层（protocol.rs）

- [x] 1.1 新建 `src-tauri/src/protocol.rs`，定义 `Protocol` 枚举（openai/anthropic/gemini，serde 小写）与 `Delta` 枚举，把 `src-tauri/src/main.rs` 的 `extract_delta` 平移为 OpenAI 分支；`cargo test` 中现有 `parses_openai_sse` 迁移到新模块后仍通过
- [x] 1.2 实现 `endpoint` / `build_body` / `auth` 对三协议的分派，含 Anthropic 顶层 `system` 与 `max_tokens`、Gemini `systemInstruction` 与 user/model 角色映射及相邻同角色合并；单测断言三种协议请求体的关键形状（JSON 路径断言，不断言整体字符串）
- [x] 1.3 实现 `parse_line` 的 Anthropic（`content_block_delta`、`message_stop`、`error`）与 Gemini（`candidates[0].content.parts[].text`）分支；单测覆盖有效增量、`[DONE]`、空行、注释行、垃圾 JSON 各返回正确的 `Delta`
- [x] 1.4 用 curl 各打一发验证协议细节（Gemini 带与不带 `alt=sse` 的响应形态、Anthropic `/v1/models`、Gemini `/v1beta/models` 的分页参数）；无可用 Key 时改以官方文档为准并在 `design.md` 的 Open Questions 记录结论

## 2. 配置与凭据

- [x] 2.1 把 `Config` 改为 `{ hotkey, active, providers: Vec<Provider> }`，新增 `Provider { id, name, protocol, base_url, model }`；`cargo check` 通过且首次运行仍生成默认 OpenAI 档案
- [x] 2.2 在 `load_config` 中实现旧结构迁移：备份 `config.json.bak` → 写回新结构 → 旧 `api-key` 凭据复制到新 id 后删除；迁移判定与转换抽成纯函数并加单测（旧结构 → 单档案、新结构 → 不动、active 失效 → 回落首项）
- [x] 2.3 `key_status` / `set_key` 增加 provider id 参数，删除档案时连带删除其凭据；`cargo test` 通过，且用 `cmdkey /list | findstr quickchat` 确认保存后条目存在、删除后消失
- [x] 2.4 `ask()` 改为读取 active 档案并经 `protocol` 模块构造请求与解析流式事件；`cargo test` 通过，缺 Key 时返回引导设置页的错误而不发请求

## 3. 网络命令

- [x] 3.1 实现 `list_models(provider) -> Result<Vec<String>, String>`：按协议分派端点（OpenAI `/models`、Anthropic `/v1/models`、Gemini `/v1beta/models`），解析 `data[].id` 与 `models[].name`（去掉 `models/` 前缀）并排序；用本地 mock 返回三种形状各验证一次
- [x] 3.2 实现 `test_connection(provider, key) -> TestResult{ok, latency_ms, model_count, message}`：先列表后 1-token 非流式对话，按 401/403、404、模型不存在、连接失败分类；对本地 mock 验证成功、401、地址不可达三种结果的文案
- [x] 3.3 把 `list_models`、`test_connection` 注册进 `generate_handler`，从 devtools 控制台调用一次确认参数与返回值结构

## 4. 设置页

- [x] 4.1 重排 `ui/index.html` 设置页：顶部服务商下拉 + 新建/删除，字段为名称、协议、接口地址、模型（`input list` + `datalist`）、API Key，底部测试按钮与状态行；静态打开页面确认 DOM 结构完整
- [x] 4.2 改 `ui/style.css`：设置内容区 `overflow-y: auto`，底部操作行固定在滚动区外；截图确认 760x440 下所有控件可达且滚动条不遮挡输入框
- [x] 4.3 `ui/app.js` 实现档案加载、切换时自动保存非凭据字段、新建、删除、Key 提交、拉取模型（失败降级为手填并提示）、测试连接结果渲染；用打桩 `__TAURI__` 在 headless Edge 中验证切换不丢改动、拉取失败提示、测试三种文案
- [x] 4.4 验证 Key 状态与设置页初始行为：未设 Key 的档案进入设置页显示"未设置"，切换档案后 Key 输入框清空、状态显示该档案自己的值

## 5. 集成与文档

- [x] 5.1 真实窗口 + 本地 mock SSE 端到端：OpenAI 兼容档案提问可流式渲染；新增一个指向不同端口的档案并切换为 active 后，mock 日志确认请求打到新端口
- [x] 5.2 更新 `README.md`：配置结构、三种协议各自的地址/模型填法、Key 按服务商存储、旧配置迁移与 `config.json.bak` 回滚方式；确认文档里的命令可直接执行
- [x] 5.3 `cargo build --release` 产出单文件 exe，启动后设置页可完整操作、问答可用，并报告最终体积