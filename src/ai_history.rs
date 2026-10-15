//! 历史对话: finished `@ai` answers, kept in `ai_history.json` next to the config
//! so they can be read again from the @ai panel.

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Config;

/// How many conversations are kept; the oldest drop off.
pub const MAX: usize = 100;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conversation {
    pub id: String,
    /// Unix seconds when the answer finished.
    pub time: u64,
    pub provider_id: String,
    /// The provider's display name when it answered.
    pub provider: String,
    pub model: String,
    pub question: String,
    /// Markdown, as streamed.
    pub answer: String,
}

impl Conversation {
    /// Whether `filter` appears in the question or the answer (ignoring case).
    pub fn matches(&self, filter: &str) -> bool {
        let filter = filter.trim().to_lowercase();
        filter.is_empty()
            || self.question.to_lowercase().contains(&filter)
            || self.answer.to_lowercase().contains(&filter)
    }

    /// The start of the answer as one line of plain text, for the list.
    pub fn preview(&self) -> String {
        let mut in_code = false;
        let line = self
            .answer
            .lines()
            .map(str::trim)
            .find(|line| {
                if line.starts_with("```") {
                    in_code = !in_code;
                    return false;
                }
                !in_code && line.chars().any(char::is_alphanumeric)
            })
            .unwrap_or("");
        let line = line.trim_start_matches(['#', '>', '-', '*', '+', ' ']);
        // "1. " list markers
        let line = match line.split_once(". ") {
            Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => rest,
            _ => line,
        };
        let plain: String = line.replace("**", "").replace('`', "");
        plain.chars().take(120).collect()
    }
}

fn path() -> PathBuf {
    Config::dir().join("ai_history.json")
}

/// Newest first.
pub fn load() -> Vec<Conversation> {
    fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(text.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

pub fn save(list: &[Conversation]) {
    let dir = Config::dir();
    let _ = fs::create_dir_all(&dir);
    let Ok(json) = serde_json::to_string_pretty(list) else {
        return;
    };
    // Write-then-rename so a crash never leaves a truncated file behind.
    let tmp = dir.join("ai_history.json.tmp");
    if fs::write(&tmp, json).is_ok() {
        let _ = fs::rename(&tmp, path());
    }
}

/// Put `c` on top. Asking the same provider the same question again replaces the
/// earlier answer instead of piling up copies.
pub fn remember(list: &mut Vec<Conversation>, c: Conversation) {
    list.retain(|old| !(old.provider_id == c.provider_id && old.question == c.question));
    list.insert(0, c);
    list.truncate(MAX);
}

/// When a conversation happened: "今天 14:32", "昨天 09:05", "10月12日 18:00", "2025年3月1日".
pub fn when(ts: u64) -> String {
    let now = crate::config::now_secs();
    let (Some(then), Some(today), Some(yesterday)) = (
        crate::win::local_time(ts),
        crate::win::local_time(now),
        crate::win::local_time(now.saturating_sub(86_400)),
    ) else {
        return crate::config::relative_time(ts);
    };
    format_when(then, today, yesterday)
}

/// (year, month, day, hour, minute) in local time.
type LocalTime = (u16, u16, u16, u16, u16);

fn format_when(then: LocalTime, today: LocalTime, yesterday: LocalTime) -> String {
    let (y, mo, d, h, mi) = then;
    let same_day = |other: LocalTime| (other.0, other.1, other.2) == (y, mo, d);
    if same_day(today) {
        format!("今天 {h:02}:{mi:02}")
    } else if same_day(yesterday) {
        format!("昨天 {h:02}:{mi:02}")
    } else if y == today.0 {
        format!("{mo}月{d}日 {h:02}:{mi:02}")
    } else {
        format!("{y}年{mo}月{d}日")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(provider: &str, question: &str, answer: &str) -> Conversation {
        Conversation {
            provider_id: provider.into(),
            question: question.into(),
            answer: answer.into(),
            ..Default::default()
        }
    }

    #[test]
    fn remembers_newest_first_without_repeats() {
        let mut list = Vec::new();
        remember(&mut list, conversation("deepseek", "a", "1"));
        remember(&mut list, conversation("deepseek", "b", "2"));
        remember(&mut list, conversation("claude", "a", "3"));
        remember(&mut list, conversation("deepseek", "a", "4"));
        let got: Vec<_> = list.iter().map(|c| c.answer.as_str()).collect();
        assert_eq!(got, ["4", "3", "2"]);
        for n in 0..MAX + 5 {
            remember(&mut list, conversation("deepseek", &n.to_string(), ""));
        }
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0].question, (MAX + 4).to_string());
    }

    #[test]
    fn previews_plain_text() {
        let c = conversation("x", "q", "\n```powershell\nGet-Thing\n```\n## **端口** 占用\n- 第二行");
        assert_eq!(c.preview(), "端口 占用");
        assert_eq!(conversation("x", "q", "1. 用 `netstat`").preview(), "用 netstat");
        assert_eq!(conversation("x", "q", "").preview(), "");
    }

    #[test]
    fn matches_question_or_answer() {
        let c = conversation("x", "查看 Port 占用", "用 netstat -ano");
        assert!(c.matches(""));
        assert!(c.matches("port"));
        assert!(c.matches("NETSTAT"));
        assert!(!c.matches("docker"));
    }

    #[test]
    fn formats_times() {
        let today = (2026, 10, 15, 20, 0);
        let yesterday = (2026, 10, 14, 20, 0);
        assert_eq!(format_when((2026, 10, 15, 9, 5), today, yesterday), "今天 09:05");
        assert_eq!(format_when((2026, 10, 14, 23, 59), today, yesterday), "昨天 23:59");
        assert_eq!(format_when((2026, 3, 1, 8, 0), today, yesterday), "3月1日 08:00");
        assert_eq!(format_when((2025, 12, 31, 8, 0), today, yesterday), "2025年12月31日");
    }
}
