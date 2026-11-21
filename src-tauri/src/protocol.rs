use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_MAX_TOKENS: u32 = 2048;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    OpenAi,
    Anthropic,
    Gemini,
}

impl Default for Protocol {
    fn default() -> Self {
        Protocol::OpenAi
    }
}

pub enum Delta {
    Text(String),
    Done,
    Fail(String),
    Skip,
}

pub fn endpoint(p: Protocol, base: &str, model: &str) -> String {
    let base = base.trim_end_matches('/');
    match p {
        Protocol::OpenAi => format!("{base}/chat/completions"),
        Protocol::Anthropic => format!("{base}/messages"),
        Protocol::Gemini => format!("{base}/models/{model}:streamGenerateContent?alt=sse"),
    }
}

pub fn models_endpoint(_p: Protocol, base: &str) -> String {
    format!("{}/models", base.trim_end_matches('/'))
}

fn system_text(messages: &[Value]) -> String {
    messages
        .iter()
        .filter(|m| m["role"] == "system")
        .filter_map(|m| m["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn build_body(p: Protocol, model: &str, messages: &[Value], stream: bool) -> Value {
    match p {
        Protocol::OpenAi => json!({ "model": model, "messages": messages, "stream": stream }),
        Protocol::Anthropic => {
            let system = system_text(messages);
            let msgs: Vec<Value> = messages
                .iter()
                .filter(|m| m["role"] != "system")
                .map(|m| json!({ "role": m["role"], "content": m["content"] }))
                .collect();
            let mut body = json!({
                "model": model,
                "max_tokens": ANTHROPIC_MAX_TOKENS,
                "messages": msgs,
                "stream": stream,
            });
            if !system.is_empty() {
                body["system"] = json!(system);
            }
            body
        }
        Protocol::Gemini => {
            let system = system_text(messages);
            let mut contents: Vec<Value> = Vec::new();
            for m in messages.iter().filter(|m| m["role"] != "system") {
                let role = if m["role"] == "assistant" { "model" } else { "user" };
                let text = m["content"].as_str().unwrap_or_default();
                match contents.last_mut() {
                    // Gemini 要求 user/model 交替，相邻同角色必须合并
                    Some(last) if last["role"] == role => {
                        let prev = last["parts"][0]["text"].as_str().unwrap_or_default();
                        last["parts"] = json!([{ "text": format!("{prev}\n{text}") }]);
                    }
                    _ => contents.push(json!({ "role": role, "parts": [{ "text": text }] })),
                }
            }
            let mut body = json!({ "contents": contents });
            if !system.is_empty() {
                body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
            }
            body
        }
    }
}

pub fn auth(p: Protocol, req: reqwest::RequestBuilder, key: &str) -> reqwest::RequestBuilder {
    match p {
        Protocol::OpenAi => req.bearer_auth(key),
        Protocol::Anthropic => req
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION),
        Protocol::Gemini => req.header("x-goog-api-key", key),
    }
}

pub fn parse_line(p: Protocol, line: &str) -> Delta {
    let Some(data) = line.trim().strip_prefix("data:") else {
        return Delta::Skip;
    };
    let data = data.trim();
    if data.is_empty() {
        return Delta::Skip;
    }
    if data == "[DONE]" {
        return Delta::Done;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return Delta::Skip;
    };
    match p {
        Protocol::OpenAi => v["choices"][0]["delta"]["content"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| Delta::Text(s.to_string()))
            .unwrap_or(Delta::Skip),
        Protocol::Anthropic => match v["type"].as_str() {
            Some("content_block_delta") => v["delta"]["text"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| Delta::Text(s.to_string()))
                .unwrap_or(Delta::Skip),
            Some("message_stop") => Delta::Done,
            Some("error") => Delta::Fail(
                v["error"]["message"]
                    .as_str()
                    .unwrap_or("上游返回错误")
                    .to_string(),
            ),
            _ => Delta::Skip,
        },
        Protocol::Gemini => {
            let text: String = v["candidates"][0]["content"]["parts"]
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|x| x["text"].as_str())
                        .collect::<String>()
                })
                .unwrap_or_default();
            if !text.is_empty() {
                Delta::Text(text)
            } else if v["candidates"][0]["finishReason"].is_string() {
                Delta::Done
            } else {
                Delta::Skip
            }
        }
    }
}

pub fn parse_models(p: Protocol, v: &Value) -> Vec<String> {
    let arr = match p {
        Protocol::Gemini => v["models"].as_array(),
        _ => v["data"].as_array(),
    };
    let Some(arr) = arr else {
        return Vec::new();
    };
    let mut out: Vec<String> = arr
        .iter()
        .filter_map(|m| match p {
            Protocol::Gemini => {
                let name = m["name"].as_str()?;
                Some(name.strip_prefix("models/").unwrap_or(name).to_string())
            }
            _ => m["id"].as_str().map(str::to_string),
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_sse() {
        assert!(matches!(
            parse_line(
                Protocol::OpenAi,
                "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}"
            ),
            Delta::Text(t) if t == "hi"
        ));
        assert!(matches!(
            parse_line(Protocol::OpenAi, "data: [DONE]"),
            Delta::Done
        ));
        assert!(matches!(
            parse_line(Protocol::OpenAi, ": keep-alive"),
            Delta::Skip
        ));
        assert!(matches!(
            parse_line(Protocol::OpenAi, "data: not-json"),
            Delta::Skip
        ));
        assert!(matches!(parse_line(Protocol::OpenAi, ""), Delta::Skip));
    }

    #[test]
    fn builds_openai_body() {
        let msgs = vec![
            json!({"role":"system","content":"sys"}),
            json!({"role":"user","content":"hi"}),
        ];
        let body = build_body(Protocol::OpenAi, "m", &msgs, true);
        assert_eq!(body["model"], "m");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(
            endpoint(Protocol::OpenAi, "https://x/v1/", "m"),
            "https://x/v1/chat/completions"
        );
    }

    #[test]
    fn builds_anthropic_body() {
        let msgs = vec![
            json!({"role":"system","content":"sys"}),
            json!({"role":"user","content":"hi"}),
            json!({"role":"assistant","content":"yo"}),
        ];
        let body = build_body(Protocol::Anthropic, "claude", &msgs, true);
        assert_eq!(body["system"], "sys");
        assert_eq!(body["max_tokens"], 2048);
        let arr = body["messages"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["role"], "user");
        assert_eq!(arr[1]["role"], "assistant");
        assert_eq!(
            endpoint(Protocol::Anthropic, "https://api.anthropic.com/v1", "c"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn builds_gemini_body() {
        let msgs = vec![
            json!({"role":"system","content":"sys"}),
            json!({"role":"user","content":"a"}),
            json!({"role":"assistant","content":"b"}),
            json!({"role":"user","content":"c"}),
            json!({"role":"user","content":"d"}),
        ];
        let body = build_body(Protocol::Gemini, "gemini-2.0-flash", &msgs, true);
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
        let arr = body["contents"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[1]["role"], "model");
        assert_eq!(arr[2]["parts"][0]["text"], "c\nd");
        assert_eq!(
            endpoint(Protocol::Gemini, "https://g/v1beta", "gemini-2.0-flash"),
            "https://g/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn parses_anthropic_sse() {
        assert!(matches!(
            parse_line(
                Protocol::Anthropic,
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hi\"}}"
            ),
            Delta::Text(t) if t == "hi"
        ));
        assert!(matches!(
            parse_line(Protocol::Anthropic, "data: {\"type\":\"message_stop\"}"),
            Delta::Done
        ));
        assert!(matches!(
            parse_line(
                Protocol::Anthropic,
                "data: {\"type\":\"error\",\"error\":{\"message\":\"overloaded\"}}"
            ),
            Delta::Fail(m) if m == "overloaded"
        ));
    }

    #[test]
    fn parses_gemini_sse() {
        assert!(matches!(
            parse_line(
                Protocol::Gemini,
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi\"}]}}]}"
            ),
            Delta::Text(t) if t == "hi"
        ));
        assert!(matches!(
            parse_line(
                Protocol::Gemini,
                "data: {\"candidates\":[{\"finishReason\":\"STOP\"}]}"
            ),
            Delta::Done
        ));
    }

    #[test]
    fn parses_model_lists() {
        let openai = json!({"data":[{"id":"b"},{"id":"a"},{"id":"a"}]});
        assert_eq!(parse_models(Protocol::OpenAi, &openai), vec!["a", "b"]);
        let anthropic = json!({"data":[{"id":"claude-x"}]});
        assert_eq!(parse_models(Protocol::Anthropic, &anthropic), vec!["claude-x"]);
        let gemini = json!({"models":[{"name":"models/gemini-pro"},{"name":"models/gemini-flash"}]});
        assert_eq!(
            parse_models(Protocol::Gemini, &gemini),
            vec!["gemini-flash", "gemini-pro"]
        );
        assert!(parse_models(Protocol::OpenAi, &json!({})).is_empty());
    }
}