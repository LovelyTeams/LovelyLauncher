//! Small fuzzy matcher with pinyin support ("wx" / "weixin" both match "微信").

use pinyin::ToPinyin;

/// Pre-computed lowercase forms of a searchable name.
#[derive(Clone, Debug, Default)]
pub struct SearchKey {
    lower: String,
    /// CamelCase split into words, e.g. "lovely mem" for "LovelyMem" (empty if unchanged).
    spaced: String,
    /// Full pinyin with ASCII kept as-is, e.g. "zhongwen notes" for "中文 notes".
    pinyin: String,
    /// First letters of each word / hanzi, e.g. "zw" for "中文".
    initials: String,
}

impl SearchKey {
    pub fn new(name: &str) -> Self {
        let lower = name.to_lowercase();
        let spaced = split_camel(name);
        let has_cjk = lower.chars().any(|c| !c.is_ascii());
        let (mut pinyin, mut initials) = (String::new(), String::new());
        let mut word_start = true;
        let words = if spaced.is_empty() { &lower } else { &spaced };
        for (ch, py) in words.chars().zip(words.as_str().to_pinyin()) {
            match py {
                Some(py) => {
                    pinyin.push_str(py.plain());
                    initials.push_str(py.first_letter());
                    word_start = true;
                }
                None => {
                    if ch.is_alphanumeric() {
                        if word_start {
                            initials.push(ch);
                        }
                        pinyin.push(ch);
                        word_start = false;
                    } else {
                        word_start = true;
                    }
                }
            }
        }
        if !has_cjk {
            pinyin.clear();
        }
        Self {
            lower,
            spaced,
            pinyin,
            initials,
        }
    }

    /// Score `query` (already lowercased) against this key; `None` = no match.
    pub fn score(&self, query: &str) -> Option<i32> {
        if query.is_empty() {
            return Some(0);
        }
        let mut best = fuzzy_score(&self.lower, query);
        if !self.spaced.is_empty() {
            best = best.max(fuzzy_score(&self.spaced, query));
        }
        if !self.pinyin.is_empty() {
            best = best.max(fuzzy_score(&self.pinyin, query).map(|s| s - 20));
        }
        if self.initials.len() >= query.len() && self.initials.starts_with(query) {
            best = best.max(Some(700 - (self.initials.len() - query.len()) as i32));
        }
        best
    }
}

/// Lowercase `name` with spaces inserted at CamelCase boundaries; empty when
/// that changes nothing.
fn split_camel(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = ch.is_lowercase();
        out.extend(ch.to_lowercase());
    }
    if out == name.to_lowercase() { String::new() } else { out }
}

/// Score a lowercase `text` against a lowercase `query`.
fn fuzzy_score(text: &str, query: &str) -> Option<i32> {
    if text == query {
        return Some(1000);
    }
    let len_penalty = text.chars().count().min(60) as i32;
    if text.starts_with(query) {
        return Some(900 - len_penalty);
    }
    if let Some(pos) = text.find(query) {
        let at_word = text[..pos]
            .chars()
            .last()
            .is_none_or(|c| !c.is_alphanumeric());
        let base = if at_word { 800 } else { 600 };
        return Some(base - pos.min(100) as i32 - len_penalty);
    }

    // Subsequence match with bonuses for word starts and consecutive chars.
    let text: Vec<char> = text.chars().collect();
    let mut score = 300;
    let mut ti = 0;
    let mut prev_match: Option<usize> = None;
    for qc in query.chars() {
        if qc == ' ' {
            continue;
        }
        let found = (ti..text.len()).find(|&i| text[i] == qc)?;
        let word_start = found == 0 || !text[found - 1].is_alphanumeric();
        if word_start {
            score += 15;
        }
        match prev_match {
            Some(p) if p + 1 == found => score += 10,
            Some(p) => score -= (found - p - 1).min(10) as i32,
            None => score -= found.min(20) as i32,
        }
        prev_match = Some(found);
        ti = found + 1;
    }
    Some(score - len_penalty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches() {
        let k = SearchKey::new("Visual Studio Code");
        assert!(k.score("vsc").is_some());
        assert!(k.score("code").is_some());
        assert!(k.score("xyz").is_none());
        assert!(k.score("visual").unwrap() > k.score("vsc").unwrap());

        let k = SearchKey::new("LovelyMem");
        assert!(k.score("mem").unwrap() >= 780, "{:?}", k.score("mem"));
        assert!(k.score("lm").is_some());

        let k = SearchKey::new("微信");
        assert!(k.score("wx").is_some());
        assert!(k.score("weixin").is_some());
        assert!(k.score("微").is_some());

        let k = SearchKey::new("任务管理器");
        assert!(k.score("rwgl").is_some());
        assert!(k.score("renwu").is_some());
    }
}
