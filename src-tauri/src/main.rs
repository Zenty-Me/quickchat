#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod protocol;

use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use protocol::{Delta, Protocol};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEFAULT_HOTKEY: &str = "Ctrl+Shift+Space";
const KEYRING_SERVICE: &str = "quickchat";
const LEGACY_KEYRING_USER: &str = "api-key";
const SYSTEM_PROMPT: &str = "用简体中文回答，简洁直接，必要时用 Markdown。";

#[derive(Serialize, Deserialize, Clone)]
struct Provider {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    protocol: Protocol,
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_model")]
    model: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    #[serde(default = "default_hotkey")]
    hotkey: String,
    #[serde(default)]
    active: String,
    #[serde(default)]
    providers: Vec<Provider>,
}

impl Default for Config {
    fn default() -> Self {
        let p = Provider::default();
        Config {
            hotkey: default_hotkey(),
            active: p.id.clone(),
            providers: vec![p],
        }
    }
}

impl Default for Provider {
    fn default() -> Self {
        Provider {
            id: new_id(),
            name: "OpenAI".into(),
            protocol: Protocol::OpenAi,
            base_url: default_base_url(),
            model: default_model(),
        }
    }
}

fn new_id() -> String {
    format!(
        "p{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.to_string()
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

fn default_hotkey() -> String {
    DEFAULT_HOTKEY.to_string()
}

#[derive(Serialize, Deserialize, Clone)]
struct Turn {
    role: String,
    content: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "t", content = "v")]
enum StreamEvent {
    Delta(String),
    Done,
    Error(String),
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

fn write_config(app: &AppHandle, config: &Config) -> Result<(), String> {
    let path = config_path(app)?;
    let raw = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, raw).map_err(|e| e.to_string())
}

/// 旧版单服务商配置转换为档案列表；第二个返回值表示是否需要回写迁移结果
fn parse_config(raw: &str) -> (Config, bool) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (Config::default(), true);
    };
    if v.get("providers").and_then(|p| p.as_array()).is_some() {
        let cfg = serde_json::from_value::<Config>(v).unwrap_or_default();
        return (normalize(cfg), false);
    }
    let base_url = v["base_url"]
        .as_str()
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string();
    let name = if base_url.contains("api.openai.com") {
        "OpenAI"
    } else {
        "默认服务商"
    };
    let p = Provider {
        id: new_id(),
        name: name.into(),
        protocol: Protocol::OpenAi,
        base_url,
        model: v["model"].as_str().unwrap_or(DEFAULT_MODEL).to_string(),
    };
    let cfg = Config {
        hotkey: v["hotkey"].as_str().unwrap_or(DEFAULT_HOTKEY).to_string(),
        active: p.id.clone(),
        providers: vec![p],
    };
    (cfg, true)
}

fn normalize(mut cfg: Config) -> Config {
    cfg.providers.retain(|p| !p.id.is_empty());
    if cfg.providers.is_empty() {
        return Config::default();
    }
    if !cfg.providers.iter().any(|p| p.id == cfg.active) {
        cfg.active = cfg.providers[0].id.clone();
    }
    cfg
}

fn migrate_legacy_key(cfg: &Config) {
    let Some(old) = keyring::Entry::new(KEYRING_SERVICE, LEGACY_KEYRING_USER)
        .ok()
        .and_then(|e| e.get_password().ok())
    else {
        return;
    };
    let Some(first) = cfg.providers.first() else {
        return;
    };
    let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, &first.id) else {
        return;
    };
    if entry.set_password(&old).is_ok() {
        if let Ok(legacy) = keyring::Entry::new(KEYRING_SERVICE, LEGACY_KEYRING_USER) {
            let _ = legacy.delete_credential();
        }
    }
}

fn load_config(app: &AppHandle) -> Result<Config, String> {
    let path = config_path(app)?;
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(Config::default());
    };
    let (cfg, migrated) = parse_config(raw.trim_start_matches('\u{feff}'));
    if migrated {
        if !raw.trim().is_empty() {
            let _ = std::fs::write(path.with_extension("json.bak"), &raw);
        }
        write_config(app, &cfg)?;
        migrate_legacy_key(&cfg);
    }
    Ok(cfg)
}

fn entry_for(provider_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, provider_id).map_err(|e| e.to_string())
}

fn read_key(provider_id: &str) -> Option<String> {
    entry_for(provider_id)
        .ok()
        .and_then(|e| e.get_password().ok())
        .or_else(|| std::env::var("QUICKCHAT_API_KEY").ok())
        .filter(|k| !k.trim().is_empty())
}

#[tauri::command]
fn get_config(app: AppHandle) -> Result<Config, String> {
    load_config(&app)
}

#[tauri::command]
fn set_config(app: AppHandle, config: Config) -> Result<(), String> {
    let config = normalize(config);
    let old = load_config(&app)?;
    for gone in old
        .providers
        .iter()
        .filter(|p| !config.providers.iter().any(|q| q.id == p.id))
    {
        if let Ok(entry) = entry_for(&gone.id) {
            let _ = entry.delete_credential();
        }
    }
    write_config(&app, &config)
}

#[tauri::command]
fn key_status(provider_id: String) -> bool {
    read_key(&provider_id).is_some()
}

#[tauri::command]
fn set_key(provider_id: String, key: String) -> Result<(), String> {
    let entry = entry_for(&provider_id)?;
    if key.trim().is_empty() {
        let _ = entry.delete_credential();
        return Ok(());
    }
    entry.set_password(key.trim()).map_err(|e| e.to_string())
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

#[tauri::command]
async fn ask(
    app: AppHandle,
    prompt: String,
    history: Vec<Turn>,
    channel: Channel<StreamEvent>,
) -> Result<(), String> {
    let cfg = load_config(&app)?;
    let Some(p) = cfg
        .providers
        .iter()
        .find(|p| p.id == cfg.active)
        .or_else(|| cfg.providers.first())
    else {
        let _ = channel.send(StreamEvent::Error("尚未配置服务商，请先在设置里添加".into()));
        return Ok(());
    };
    let key = match read_key(&p.id) {
        Some(k) => k,
        None => {
            let _ = channel.send(StreamEvent::Error(
                "尚未设置 API Key，请先在设置里填写".into(),
            ));
            return Ok(());
        }
    };

    let mut messages = vec![serde_json::json!({ "role": "system", "content": SYSTEM_PROMPT })];
    for t in &history {
        messages.push(serde_json::json!({ "role": t.role, "content": t.content }));
    }
    messages.push(serde_json::json!({ "role": "user", "content": prompt }));

    let url = protocol::endpoint(p.protocol, &p.base_url, &p.model);
    let body = protocol::build_body(p.protocol, &p.model, &messages, true);

    let resp = match protocol::auth(p.protocol, reqwest::Client::new().post(&url), &key)
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = channel.send(StreamEvent::Error(format!("请求失败: {e}")));
            return Ok(());
        }
    };

    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let _ = channel.send(StreamEvent::Error(format!("HTTP {code}: {}", snippet(&text))));
        return Ok(());
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else { break };
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf.drain(..=pos).collect();
            match protocol::parse_line(p.protocol, &line) {
                Delta::Text(t) => {
                    let _ = channel.send(StreamEvent::Delta(t));
                }
                Delta::Done => {
                    let _ = channel.send(StreamEvent::Done);
                    return Ok(());
                }
                Delta::Fail(msg) => {
                    let _ = channel.send(StreamEvent::Error(msg));
                    return Ok(());
                }
                Delta::Skip => {}
            }
        }
    }
    let _ = channel.send(StreamEvent::Done);
    Ok(())
}

fn snippet(text: &str) -> String {
    text.chars().take(300).collect()
}

// shortcut: 只取第一页，模型数超过一页上限（Gemini 默认 50）时列表不全，需要时再加分页
async fn fetch_models(p: &Provider, key: &str) -> Result<Vec<String>, String> {
    let url = protocol::models_endpoint(p.protocol, &p.base_url);
    let resp = protocol::auth(p.protocol, reqwest::Client::new().get(&url), key)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|_| format!("无法连接接口地址 · {url}"))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}: {}", snippet(&text)));
    }
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("返回的不是 JSON: {}", snippet(&text)))?;
    let list = protocol::parse_models(p.protocol, &v);
    if list.is_empty() {
        return Err("接口未返回模型列表".into());
    }
    Ok(list)
}

enum ProbeError {
    Net(String),
    Http(u16, String),
}

async fn probe_chat(p: &Provider, key: &str) -> Result<(), ProbeError> {
    let msgs = vec![serde_json::json!({ "role": "user", "content": "ping" })];
    let mut body = protocol::build_body(p.protocol, &p.model, &msgs, false);
    match p.protocol {
        Protocol::Gemini => body["generationConfig"] = serde_json::json!({ "maxOutputTokens": 1 }),
        _ => body["max_tokens"] = serde_json::json!(1),
    }
    let url = protocol::endpoint(p.protocol, &p.base_url, &p.model);
    let resp = protocol::auth(p.protocol, reqwest::Client::new().post(&url), key)
        .timeout(Duration::from_secs(12))
        .json(&body)
        .send()
        .await
        .map_err(|e| ProbeError::Net(e.to_string()))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(ProbeError::Http(status, text))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestResult {
    ok: bool,
    latency_ms: u64,
    model_count: Option<usize>,
    message: String,
}

fn pick_key(provider: &Provider, key: Option<String>) -> Option<String> {
    key.map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .or_else(|| read_key(&provider.id))
}

#[tauri::command]
async fn list_models(provider: Provider, key: Option<String>) -> Result<Vec<String>, String> {
    let key = pick_key(&provider, key).ok_or("未设置 API Key")?;
    fetch_models(&provider, &key).await
}

#[tauri::command]
async fn test_connection(provider: Provider, key: Option<String>) -> Result<TestResult, String> {
    let Some(key) = pick_key(&provider, key) else {
        return Ok(TestResult {
            ok: false,
            latency_ms: 0,
            model_count: None,
            message: "未设置 API Key".into(),
        });
    };
    let started = std::time::Instant::now();
    let latency = |t: std::time::Instant| t.elapsed().as_millis() as u64;

    match fetch_models(&provider, &key).await {
        Ok(list) => {
            return Ok(TestResult {
                ok: true,
                latency_ms: latency(started),
                model_count: Some(list.len()),
                message: format!("认证通过 · {} 个模型", list.len()),
            })
        }
        // 连不上就不必再试一次对话，省掉一轮等待
        Err(e) if e.starts_with("无法连接接口地址") => {
            return Ok(TestResult {
                ok: false,
                latency_ms: latency(started),
                model_count: None,
                message: e,
            })
        }
        Err(_) => {}
    }

    let (ok, message) = match probe_chat(&provider, &key).await {
        Ok(()) => (true, "认证通过 · 模型可用".to_string()),
        Err(ProbeError::Net(e)) => (false, format!("无法连接接口地址 · {e}")),
        Err(ProbeError::Http(code, body)) => (
            false,
            match code {
                401 | 403 => format!("Key 无效 · HTTP {code}"),
                404 => format!("接口地址不对 · HTTP {code}"),
                400 if body.to_lowercase().contains("model") => {
                    format!("模型名不存在 · HTTP {code}")
                }
                _ => format!("HTTP {code}: {}", snippet(&body)),
            },
        ),
    };
    Ok(TestResult {
        ok,
        latency_ms: latency(started),
        model_count: None,
        message,
    })
}

#[cfg(windows)]
fn foreground_is_ours() -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> *mut std::ffi::c_void;
        fn GetWindowThreadProcessId(hwnd: *mut std::ffi::c_void, pid: *mut u32) -> u32;
    }
    unsafe {
        let h = GetForegroundWindow();
        if h.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        pid == std::process::id()
    }
}

#[cfg(not(windows))]
fn foreground_is_ours() -> bool {
    false
}

fn toggle_main(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let visible = win.is_visible().unwrap_or(false);
    if visible {
        let _ = win.hide();
        return;
    }

    let prefill = arboard::Clipboard::new()
        .ok()
        .and_then(|mut c| c.get_text().ok())
        .filter(|t| !t.trim().is_empty() && t.chars().count() <= 4000);

    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit("prefill", prefill);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_main(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_config,
            key_status,
            set_key,
            hide_window,
            ask,
            list_models,
            test_connection
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            let hotkey = load_config(&handle)
                .map(|c| c.hotkey)
                .unwrap_or_else(|_| default_hotkey());
            let shortcut = Shortcut::from_str(&hotkey).unwrap_or_else(|_| {
                Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space)
            });
            let _ = app.global_shortcut().register(shortcut);

            let first_run = config_path(&handle).map(|p| !p.exists()).unwrap_or(false);

            if let Some(win) = app.get_webview_window("main") {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if matches!(event, WindowEvent::Focused(false)) {
                        let w = w.clone();
                        std::thread::spawn(move || {
                            // 原生 select 弹层会抢焦点，立刻隐藏会让下拉一打开窗口就消失
                            std::thread::sleep(std::time::Duration::from_millis(250));
                            if !w.is_focused().unwrap_or(false) && !foreground_is_ours() {
                                let _ = w.hide();
                            }
                        });
                    }
                });
                if first_run {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }

            let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
            let show = MenuItem::with_id(app, "show", "显示", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let auto = CheckMenuItem::with_id(app, "auto", "开机自启", true, autostart_on, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &show,
                    &settings,
                    &PredefinedMenuItem::separator(app)?,
                    &auto,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?;

            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => toggle_main(app),
                    "settings" => {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                            let _ = app.emit("goto-settings", ());
                        }
                    }
                    "auto" => {
                        let launcher = app.autolaunch();
                        let on = launcher.is_enabled().unwrap_or(false);
                        let _ = if on { launcher.disable() } else { launcher.enable() };
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running quickchat");
}

fn main() {
    run();
}

#[cfg(test)]
mod tests {
    use super::{parse_config, Protocol};

    #[test]
    fn migrates_legacy_config() {
        let raw = r#"{"base_url":"https://api.deepseek.com/v1/","model":"deepseek-chat","hotkey":"Ctrl+Alt+Q"}"#;
        let (cfg, migrated) = parse_config(raw);
        assert!(migrated);
        assert_eq!(cfg.hotkey, "Ctrl+Alt+Q");
        assert_eq!(cfg.providers.len(), 1);
        let p = &cfg.providers[0];
        assert_eq!(cfg.active, p.id);
        assert_eq!(p.base_url, "https://api.deepseek.com/v1");
        assert_eq!(p.model, "deepseek-chat");
        assert_eq!(p.protocol, Protocol::OpenAi);
    }

    #[test]
    fn keeps_provider_config_without_migration() {
        let raw = r#"{"hotkey":"Ctrl+Space","active":"p2","providers":[{"id":"p1","name":"A","protocol":"anthropic","base_url":"https://api.anthropic.com/v1","model":"claude"},{"id":"p2","name":"B","protocol":"gemini","base_url":"https://g/v1beta","model":"gemini-2.0-flash"}]}"#;
        let (cfg, migrated) = parse_config(raw);
        assert!(!migrated);
        assert_eq!(cfg.active, "p2");
        assert_eq!(cfg.providers[0].protocol, Protocol::Anthropic);
        assert_eq!(cfg.providers[1].protocol, Protocol::Gemini);
    }

    #[test]
    fn normalizes_dangling_active() {
        let raw = r#"{"hotkey":"Ctrl+Space","active":"gone","providers":[{"id":"p1","name":"A"}]}"#;
        let (cfg, _) = parse_config(raw);
        assert_eq!(cfg.active, "p1");
    }

    #[test]
    fn falls_back_to_default_provider() {
        let (cfg, _) = parse_config(r#"{"hotkey":"Ctrl+Space","active":"","providers":[]}"#);
        assert_eq!(cfg.providers.len(), 1);
        assert_eq!(cfg.active, cfg.providers[0].id);
        assert_eq!(cfg.providers[0].protocol, Protocol::OpenAi);
    }
}