//! `@ai` quick questions: model listing and streamed answers for OpenAI-compatible
//! chat APIs and Anthropic's Messages API, over raw HTTPS (no SDK for Rust).
//! Blocking; run on a background thread.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::{
    config::{AiApi, AiProvider},
    http,
};

const SYSTEM_PROMPT: &str = "你是 LovelyLauncher 里的快速问答助手。用户在启动器的输入框里随手提问，\
请直接给出答案，简洁、准确，默认用中文回答；需要代码时给出可直接使用的代码块。";

const ANTHROPIC_VERSION: &str = "2023-06-01";

fn endpoint(provider: &AiProvider, path: &str) -> String {
    format!("{}/{}", provider.base_url.trim().trim_end_matches('/'), path)
}

fn auth_headers(provider: &AiProvider) -> Vec<(&'static str, String)> {
    let key = provider.api_key.trim().to_string();
    match provider.api {
        AiApi::OpenAi => vec![("Authorization", format!("Bearer {key}"))],
        AiApi::Anthropic => vec![
            ("x-api-key", key),
            ("anthropic-version", ANTHROPIC_VERSION.to_string()),
        ],
    }
}

/// Pull a readable message out of an error body (`{"error":{"message":..}}` and friends).
fn error_message(status: u32, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let detail = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("error"))
                .and_then(|m| m.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| text.chars().take(200).collect());
    let hint = match status {
        401 | 403 => "（请检查 API Key）",
        404 => "（请检查接口地址或模型名称）",
        429 => "（请求太频繁或额度不足）",
        _ => "",
    };
    format!("HTTP {status}{hint}：{detail}")
}

/// Fetch the provider's model ids (`GET {base}/models`).
pub fn list_models(provider: &AiProvider) -> Result<Vec<String>, String> {
    if provider.api_key.trim().is_empty() {
        return Err("请先填写 API Key".into());
    }
    let mut url = endpoint(provider, "models");
    if provider.api == AiApi::Anthropic {
        url.push_str("?limit=1000");
    }
    let headers = auth_headers(provider);
    let header_refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let (status, body) = http::fetch(
        &http::Request {
            method: "GET",
            url: &url,
            headers: &header_refs,
            body: None,
            receive_timeout_ms: 30_000,
        },
        8 * 1024 * 1024,
    )?;
    if status != 200 {
        return Err(error_message(status, &body));
    }
    let value: Value = serde_json::from_slice(&body).map_err(|e| format!("无法解析模型列表：{e}"))?;
    let mut models: Vec<String> = value
        .get("data")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if models.is_empty() {
        return Err("接口没有返回任何模型，可以直接手动填写模型名称".into());
    }
    models.sort();
    models.dedup();
    Ok(models)
}

/// Ask `question`, calling `on_text` with each streamed piece of the answer.
pub fn ask(
    provider: &AiProvider,
    question: &str,
    cancel: &AtomicBool,
    mut on_text: impl FnMut(&str),
) -> Result<(), String> {
    if provider.api_key.trim().is_empty() {
        return Err(format!("还没有配置 {} 的 API Key，请在 设置 → AI 助手 中填写", provider.name));
    }
    if provider.model.trim().is_empty() {
        return Err(format!("还没有为 {} 选择模型，请在 设置 → AI 助手 中拉取或填写", provider.name));
    }
    let model = provider.model.trim();
    let mut headers = auth_headers(provider);
    headers.push(("Content-Type", "application/json".into()));
    headers.push(("Accept", "text/event-stream".into()));

    let official_anthropic = provider.api == AiApi::Anthropic
        && http::parse_url(&provider.base_url).is_ok_and(|u| u.host == "api.anthropic.com");
    let (url, body) = match provider.api {
        AiApi::OpenAi => (
            endpoint(provider, "chat/completions"),
            json!({
                "model": model,
                "stream": true,
                "messages": [
                    {"role": "system", "content": SYSTEM_PROMPT},
                    {"role": "user", "content": question},
                ],
            }),
        ),
        AiApi::Anthropic => {
            let mut body = json!({
                "model": model,
                "max_tokens": 64000,
                "stream": true,
                "system": SYSTEM_PROMPT,
                "messages": [{"role": "user", "content": question}],
            });
            if official_anthropic {
                // If a safety classifier declines, let the API re-route instead of just stopping.
                body["fallbacks"] = json!("default");
                headers.push(("anthropic-beta", "server-side-fallback-2026-07-01".into()));
            }
            (endpoint(provider, "messages"), body)
        }
    };
    let body = body.to_string();
    let header_refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();

    let status = std::cell::Cell::new(0u32);
    let mut error_body = Vec::new();
    let mut parser = SseParser::default();
    let mut stream_error: Option<String> = None;
    let mut refused = false;
    http::send(
        &http::Request {
            method: "POST",
            url: &url,
            headers: &header_refs,
            body: Some(body.as_bytes()),
            receive_timeout_ms: 120_000,
        },
        |head| status.set(head.status),
        |chunk| {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            if status.get() != 200 {
                error_body.extend_from_slice(chunk);
                return error_body.len() < 64 * 1024;
            }
            for data in parser.push(chunk) {
                match parse_event(provider.api, &data) {
                    Event::Text(text) => on_text(&text),
                    Event::Error(err) => stream_error = Some(err),
                    Event::Refusal => refused = true,
                    Event::Done | Event::Other => {}
                }
            }
            stream_error.is_none()
        },
    )?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    if status.get() != 200 {
        return Err(error_message(status.get(), &error_body));
    }
    if let Some(err) = stream_error {
        return Err(err);
    }
    if refused {
        on_text("\n\n（模型拒绝回答了这个问题）");
    }
    Ok(())
}

enum Event {
    Text(String),
    Error(String),
    Refusal,
    Done,
    Other,
}

fn parse_event(api: AiApi, data: &str) -> Event {
    if data == "[DONE]" {
        return Event::Done;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return Event::Other;
    };
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        return Event::Error(msg);
    }
    match api {
        AiApi::OpenAi => v
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(|t| Event::Text(t.to_string()))
            .unwrap_or(Event::Other),
        AiApi::Anthropic => match v.get("type").and_then(Value::as_str) {
            Some("content_block_delta") => v
                .pointer("/delta/text")
                .and_then(Value::as_str)
                .map(|t| Event::Text(t.to_string()))
                .unwrap_or(Event::Other),
            Some("message_delta")
                if v.pointer("/delta/stop_reason").and_then(Value::as_str) == Some("refusal") =>
            {
                Event::Refusal
            }
            Some("message_stop") => Event::Done,
            _ => Event::Other,
        },
    }
}

/// Splits a byte stream into server-sent-event `data:` payloads, tolerating
/// chunks that end mid-line or mid-UTF-8 character.
#[derive(Default)]
struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(data) = line.strip_prefix("data:") {
                out.push(data.trim_start().to_string());
            }
        }
        out
    }
}

/// `@ai 问题` / `@deepseek 问题` → (provider id or "ai", question).
pub fn parse_prefix(query: &str) -> Option<(String, String)> {
    let rest = query.trim_start().strip_prefix('@')?;
    let end = rest
        .find(|c: char| c.is_whitespace())
        .unwrap_or(rest.len());
    let id = rest[..end].to_ascii_lowercase();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    Some((id, rest[end..].trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_split_across_chunks() {
        let mut p = SseParser::default();
        assert!(p.push(b"event: x\ndata: {\"a\"").is_empty());
        let got = p.push(b":1}\r\n\ndata: [DONE]\n");
        assert_eq!(got, vec!["{\"a\":1}", "[DONE]"]);
        // UTF-8 split in the middle of a character
        let bytes = "data: 你好\n".as_bytes();
        assert!(p.push(&bytes[..8]).is_empty());
        assert_eq!(p.push(&bytes[8..]), vec!["你好"]);
    }

    #[test]
    fn events() {
        let t = parse_event(AiApi::OpenAi, r#"{"choices":[{"delta":{"content":"hi"}}]}"#);
        assert!(matches!(t, Event::Text(ref s) if s == "hi"));
        let t = parse_event(
            AiApi::Anthropic,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"yo"}}"#,
        );
        assert!(matches!(t, Event::Text(ref s) if s == "yo"));
        assert!(matches!(
            parse_event(AiApi::Anthropic, r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#),
            Event::Error(ref m) if m == "busy"
        ));
    }

    #[test]
    fn prefixes() {
        assert_eq!(parse_prefix("@ai 你好"), Some(("ai".into(), "你好".into())));
        assert_eq!(parse_prefix("@DeepSeek  what"), Some(("deepseek".into(), "what".into())));
        assert_eq!(parse_prefix("@ai"), Some(("ai".into(), String::new())));
        assert_eq!(parse_prefix("hello"), None);
        assert_eq!(parse_prefix("@ 你好"), None);
    }
}
