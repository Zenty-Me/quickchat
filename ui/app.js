const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const qEl = $("q");
const answerEl = $("answer");
const actionsEl = $("actions");
const statusEl = $("status");
const askView = $("ask");
const settingsView = $("settings");

let config = { hotkey: "", active: "", providers: [] };
let history = [];
let streaming = false;
let lastAnswer = "";
let lastPrompt = "";
let delArmed = false;

// 新建服务商时的预设；已保存的档案才是真正在用的配置
const PRESETS = [
  { name: "OpenAI", protocol: "openai", base_url: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  { name: "DeepSeek", protocol: "openai", base_url: "https://api.deepseek.com/v1", model: "deepseek-chat" },
  { name: "通义千问", protocol: "openai", base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1", model: "qwen-plus" },
  { name: "Kimi", protocol: "openai", base_url: "https://api.moonshot.cn/v1", model: "moonshot-v1-8k" },
  { name: "智谱 GLM", protocol: "openai", base_url: "https://open.bigmodel.cn/api/paas/v4", model: "glm-4-flash" },
  { name: "SiliconFlow", protocol: "openai", base_url: "https://api.siliconflow.cn/v1", model: "Qwen/Qwen2.5-7B-Instruct" },
  { name: "OpenRouter", protocol: "openai", base_url: "https://openrouter.ai/api/v1", model: "openai/gpt-4o-mini" },
  { name: "Anthropic", protocol: "anthropic", base_url: "https://api.anthropic.com/v1", model: "claude-sonnet-4-5" },
  { name: "Gemini", protocol: "gemini", base_url: "https://generativelanguage.googleapis.com/v1beta", model: "gemini-2.0-flash" },
  { name: "Ollama 本地", protocol: "openai", base_url: "http://127.0.0.1:11434/v1", model: "llama3.2" },
  { name: "自定义", protocol: "openai", base_url: "", model: "" },
];

const newId = () => "p" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
const activeProvider = () => config.providers.find((p) => p.id === config.active);

const ESC = (s) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

function md(src) {
  const blocks = [];
  let text = ESC(src).replace(/```(\w*)\n([\s\S]*?)```/g, (_, lang, code) => {
    blocks.push(`<pre><code>${code.replace(/\n$/, "")}</code></pre>`);
    return `\u0000${blocks.length - 1}\u0000`;
  });

  text = text
    .replace(/`([^`\n]+)`/g, "<code>$1</code>")
    .replace(/^###\s+(.*)$/gm, "<h3>$1</h3>")
    .replace(/^##\s+(.*)$/gm, "<h2>$1</h2>")
    .replace(/^#\s+(.*)$/gm, "<h1>$1</h1>")
    .replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[\s(])\*([^*\n]+)\*/g, "$1<em>$2</em>")
    .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" target="_blank">$1</a>');

  text = text.replace(/(?:^|\n)((?:(?:[-*]|\d+\.)\s+.*(?:\n|$))+)/g, (m, list) => {
    const lines = list.trim().split("\n");
    const tag = /^\d+\./.test(lines[0]) ? "ol" : "ul";
    const items = lines.map((l) => `<li>${l.replace(/^(?:[-*]|\d+\.)\s+/, "")}</li>`).join("");
    return `\n<${tag}>${items}</${tag}>`;
  });

  text = text.replace(/(?:^|\n)((?:&gt;\s?.*(?:\n|$))+)/g, (m, q) => {
    const body = q.trim().split("\n").map((l) => l.replace(/^&gt;\s?/, "")).join("<br>");
    return `\n<blockquote>${body}</blockquote>`;
  });

  text = text
    .split(/\n{2,}/)
    .map((p) =>
      /^\s*<(!?h\d|ul|ol|pre|blockquote)/.test(p) || /^\s*\u0000\d+\u0000\s*$/.test(p)
        ? p
        : `<p>${p.replace(/\n/g, "<br>")}</p>`,
    )
    .join("");

  return text.replace(/\u0000(\d+)\u0000/g, (_, i) => blocks[Number(i)]);
}

function render(text) {
  answerEl.hidden = false;
  answerEl.innerHTML = md(text);
  answerEl.scrollTop = answerEl.scrollHeight;
}

function resetView() {
  answerEl.hidden = true;
  answerEl.textContent = "";
  actionsEl.hidden = true;
  lastAnswer = "";
  lastPrompt = "";
  history = [];
  statusEl.textContent = "Enter 发送 · Shift+Enter 换行 · Esc 隐藏";
}

function setBusy(busy) {
  streaming = busy;
  statusEl.textContent = busy ? "生成中…" : "Enter 发送 · Shift+Enter 换行 · Esc 隐藏";
}

async function send() {
  const prompt = qEl.value.trim();
  if (!prompt || streaming) return;

  lastPrompt = prompt;
  qEl.value = "";
  answerEl.hidden = false;
  answerEl.textContent = "";
  actionsEl.hidden = true;
  lastAnswer = "";
  setBusy(true);

  const channel = new Channel();
  channel.onmessage = (ev) => {
    if (ev.t === "Delta") {
      lastAnswer += ev.v;
      render(lastAnswer);
    } else if (ev.t === "Done") {
      setBusy(false);
      actionsEl.hidden = false;
      if (!lastAnswer) render("（空回答）");
    } else if (ev.t === "Error") {
      setBusy(false);
      answerEl.hidden = false;
      answerEl.innerHTML = `<p class="err">${ESC(ev.v)}</p>`;
      if (/API Key|服务商/.test(ev.v)) showSettings();
    }
  };

  try {
    await invoke("ask", { prompt, history, channel });
  } catch (e) {
    setBusy(false);
    answerEl.hidden = false;
    answerEl.innerHTML = `<p class="err">${ESC(String(e))}</p>`;
  }
}

function copyText(text) {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  try {
    document.execCommand("copy");
  } finally {
    ta.remove();
  }
}

function fillProviderSelect() {
  const sel = $("s-provider");
  sel.textContent = "";
  const saved = document.createElement("optgroup");
  saved.label = "已保存";
  for (const p of config.providers) {
    saved.append(new Option(p.name || p.base_url || "未命名", p.id));
  }
  sel.append(saved);

  const add = document.createElement("optgroup");
  add.label = "新建服务商";
  PRESETS.forEach((p, i) => add.append(new Option(`＋ ${p.name}`, "new:" + i)));
  sel.append(add);

  sel.value = config.active;
}

function setTestState(text, kind) {
  const el = $("test-state");
  el.className = kind || "muted";
  el.textContent = text;
}

const PROTO_HINT = {
  openai: { base: "https://api.openai.com/v1", model: "gpt-4o-mini" },
  anthropic: { base: "https://api.anthropic.com/v1", model: "claude-sonnet-4-5" },
  gemini: { base: "https://generativelanguage.googleapis.com/v1beta", model: "gemini-2.0-flash" },
};

function fillHints() {
  const h = PROTO_HINT[$("s-protocol").value] || PROTO_HINT.openai;
  $("s-base").placeholder = h.base;
  $("s-model").placeholder = h.model;
}

function loadForm(p) {
  $("s-provider").value = p.id;
  $("s-name").value = p.name;
  $("s-protocol").value = p.protocol;
  $("s-base").value = p.base_url;
  $("s-model").value = p.model;
  $("s-key").value = "";
  $("model-list").textContent = "";
  fillHints();
  setTestState("", "muted");
  invoke("key_status", { providerId: p.id }).then((ok) => {
    $("key-state").textContent = ok ? "· 已保存" : "· 未设置";
    $("s-key").placeholder = ok ? "已保存，留空则不修改" : "粘贴 API Key";
  });
}

function commitDraft() {
  const p = activeProvider();
  if (!p) return null;
  p.name = $("s-name").value.trim() || p.name || "未命名";
  p.protocol = $("s-protocol").value;
  p.base_url = $("s-base").value.trim();
  p.model = $("s-model").value.trim();
  return p;
}

function showSettings() {
  askView.hidden = true;
  settingsView.hidden = false;
  fillProviderSelect();
  const p = activeProvider();
  if (p) loadForm(p);
}

function showAsk() {
  settingsView.hidden = true;
  askView.hidden = false;
  qEl.focus();
}

async function loadConfig() {
  config = await invoke("get_config");
  const p = activeProvider();
  const ok = p ? await invoke("key_status", { providerId: p.id }) : false;
  if (!ok) showSettings();
}

$("close").onclick = () => invoke("hide_window");
$("settings-btn").onclick = showSettings;
$("back").onclick = showAsk;
$("s-protocol").onchange = fillHints;

$("s-provider").onchange = async () => {
  const v = $("s-provider").value;
  if (v.startsWith("new:")) {
    const preset = PRESETS[Number(v.slice(4))];
    commitDraft();
    const p = {
      id: newId(),
      name: preset.name,
      protocol: preset.protocol,
      base_url: preset.base_url,
      model: preset.model,
    };
    config.providers.push(p);
    config.active = p.id;
    await invoke("set_config", { config });
    fillProviderSelect();
    loadForm(p);
    return;
  }
  commitDraft();
  config.active = v;
  await invoke("set_config", { config });
  loadForm(activeProvider());
};

$("s-del").onclick = async () => {
  if (config.providers.length <= 1) {
    setTestState("至少保留一个服务商", "err");
    return;
  }
  if (!delArmed) {
    delArmed = true;
    $("s-del").textContent = "再点一次确认删除";
    setTimeout(() => {
      delArmed = false;
      $("s-del").textContent = "删除此服务商";
    }, 3000);
    return;
  }
  delArmed = false;
  $("s-del").textContent = "删除此服务商";
  config.providers = config.providers.filter((p) => p.id !== config.active);
  config.active = config.providers[0].id;
  await invoke("set_config", { config });
  fillProviderSelect();
  loadForm(activeProvider());
};

$("save").onclick = async () => {
  const p = commitDraft();
  if (!p) return;
  if (!p.base_url || !p.model) {
    setTestState("接口地址和模型不能为空", "err");
    return;
  }
  await invoke("set_config", { config });
  const key = $("s-key").value.trim();
  if (key) {
    await invoke("set_key", { providerId: p.id, key });
    $("s-key").value = "";
  }
  const ok = await invoke("key_status", { providerId: p.id });
  $("key-state").textContent = ok ? "· 已保存" : "· 未设置";
  $("s-key").placeholder = ok ? "已保存，留空则不修改" : "粘贴 API Key";
  showAsk();
  statusEl.textContent = "已保存";
};

$("test").onclick = async () => {
  const p = commitDraft();
  if (!p) return;
  setTestState("测试中…");
  try {
    const r = await invoke("test_connection", {
      provider: p,
      key: $("s-key").value.trim(),
    });
    const ms = r.ok && r.latencyMs ? ` · ${r.latencyMs}ms` : "";
    setTestState(`${r.ok ? "✓" : "✕"} ${r.message}${ms}`, r.ok ? "ok" : "err");
  } catch (e) {
    setTestState(String(e), "err");
  }
};

$("s-models").onclick = async () => {
  const p = commitDraft();
  if (!p) return;
  const btn = $("s-models");
  btn.disabled = true;
  setTestState("拉取中…");
  try {
    const list = await invoke("list_models", {
      provider: p,
      key: $("s-key").value.trim(),
    });
    const dl = $("model-list");
    dl.textContent = "";
    for (const m of list) dl.append(new Option(m, m));
    setTestState(`已获取 ${list.length} 个模型，点模型框可挑选`, "ok");
  } catch (e) {
    $("model-list").textContent = "";
    setTestState(`无法获取列表，请手动填写模型名 · ${String(e).slice(0, 80)}`, "err");
  } finally {
    btn.disabled = false;
  }
};

$("copy").onclick = () => {
  copyText(lastAnswer);
  statusEl.textContent = "已复制";
  setTimeout(() => setBusy(false), 1200);
};

$("followup").onclick = () => {
  if (!lastAnswer) return;
  history = [
    ...history,
    { role: "user", content: lastPrompt },
    { role: "assistant", content: lastAnswer },
  ].slice(-6);
  answerEl.hidden = true;
  answerEl.textContent = "";
  actionsEl.hidden = true;
  qEl.focus();
};

$("new").onclick = () => {
  resetView();
  qEl.value = "";
  qEl.focus();
};

qEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    send();
  }
});

qEl.addEventListener("input", () => {
  qEl.style.height = "auto";
  qEl.style.height = Math.min(qEl.scrollHeight, 150) + "px";
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    invoke("hide_window");
  }
});

listen("prefill", (event) => {
  const text = event.payload;
  if (typeof text === "string" && text.trim()) {
    resetView();
    qEl.value = text.trim();
    qEl.style.height = "auto";
    qEl.style.height = Math.min(qEl.scrollHeight, 150) + "px";
  }
  showAsk();
  qEl.focus();
});

listen("goto-settings", () => {
  showSettings();
});

loadConfig();