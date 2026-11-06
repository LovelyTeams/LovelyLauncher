//! Configurable HTTP search, field mapping and pagination for any provider.
use crate::http;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub keyword: String,
    pub enabled: bool,
    pub global: bool,
    pub url: String,
    pub method: String,
    pub request: String,
    pub results_path: String,
    pub id_path: String,
    pub title_path: String,
    pub subtitle_path: String,
    pub snippet_path: String,
    pub open_url: String,
    pub activate_url: String,
    pub pagination: String,
    pub first_page: usize,
    pub total_path: String,
    pub has_more_path: String,
    pub next_cursor_path: String,
    pub auth_header: String,
    pub auth_scheme: String,
    /// Windows DPAPI encrypted UTF-8 bytes, hex encoded. Never a plaintext API key.
    pub secret: String,
    pub limit: usize,
    pub timeout_ms: i32,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            id: crate::config::new_id(),
            name: "自定义搜索".into(),
            keyword: "ext".into(),
            enabled: true,
            global: true,
            url: "http://127.0.0.1:8080/search".into(),
            method: "POST".into(),
            request: r#"{"query":"{query}","limit":"{limit}","page":"{page}"}"#.into(),
            results_path: "results".into(),
            id_path: "id".into(),
            title_path: "title".into(),
            subtitle_path: "subtitle".into(),
            snippet_path: "snippet".into(),
            open_url: String::new(),
            activate_url: "http://127.0.0.1:8080/activate".into(),
            pagination: "none".into(),
            first_page: 1,
            total_path: "total".into(),
            has_more_path: "has_more".into(),
            next_cursor_path: "next_cursor".into(),
            auth_header: "Authorization".into(),
            auth_scheme: "Bearer".into(),
            secret: String::new(),
            limit: 10,
            timeout_ms: 2000,
        }
    }
}

pub fn validate(source: &Source, others: &[Source]) -> Result<(), String> {
    if source.name.trim().is_empty() || source.keyword.trim().is_empty() || source.id.is_empty() {
        return Err("名称和搜索前缀不能为空".into());
    }
    if source
        .keyword
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && c != '-')
        || ["cb", "bw", "tx", "ai"].contains(&source.keyword.to_ascii_lowercase().as_str())
    {
        return Err("前缀只能使用英文字母、数字和连字符，且不能占用 cb / bw / tx / ai".into());
    }
    if others
        .iter()
        .any(|s| s.id != source.id && s.keyword.eq_ignore_ascii_case(&source.keyword))
    {
        return Err("该前缀已经被另一个搜索源使用".into());
    }
    if !["GET", "POST"].contains(&source.method.as_str()) {
        return Err("请求方式只支持 GET 或 POST".into());
    }
    if source.limit == 0 || source.limit > 50 || !(200..=10000).contains(&source.timeout_ms) {
        return Err("结果数须为 1～50，超时须为 200～10000 毫秒".into());
    }
    validate_url(&source.url)?;
    if !source.activate_url.is_empty() {
        validate_url(&source.activate_url)?;
    }
    if !["none", "page", "offset", "cursor"].contains(&source.pagination.as_str())
        || source.first_page > 1
    {
        return Err("分页方式须为 none / page / offset / cursor，起始页须为 0 或 1".into());
    }
    if source.pagination == "cursor" && source.next_cursor_path.is_empty() {
        return Err("游标分页需要设置下一页游标字段路径".into());
    }
    let marker = match source.pagination.as_str() {
        "page" => "{page}",
        "offset" => "{offset}",
        "cursor" => "{cursor}",
        _ => "",
    };
    let template = if source.method == "GET" {
        &source.url
    } else {
        &source.request
    };
    if !marker.is_empty() && !template.contains(marker) {
        return Err(format!("分页请求中缺少 {marker} 占位符"));
    }
    if source.auth_header.is_empty()
        || !source
            .auth_header
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        || source.auth_scheme.contains(['\r', '\n'])
    {
        return Err("认证请求头无效".into());
    }
    if !source.request.trim().is_empty() {
        serde_json::from_str::<Value>(&source.request).map_err(|_| "请求正文必须是合法 JSON")?;
    }
    if source.open_url.is_empty() && source.activate_url.is_empty() {
        return Err("请设置打开链接模板或激活接口地址".into());
    }
    if !source.open_url.is_empty() && !safe_open_url(&source.open_url) {
        return Err("打开模板需要使用 http(s) 或已注册的软件协议地址".into());
    }
    Ok(())
}

fn validate_url(url: &str) -> Result<(), String> {
    let p = http::parse_url(url)?;
    if p.host.contains(['@', '\\', '#', ' ']) || url.contains(['\r', '\n']) {
        return Err("接口地址无效".into());
    }
    if !p.secure && !["127.0.0.1", "localhost"].contains(&p.host.as_str()) {
        return Err("非本机 HTTP 地址请改用 HTTPS".into());
    }
    Ok(())
}

fn safe_open_url(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once("://") else {
        return false;
    };
    !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ['+', '-', '.'].contains(&c))
        && !["file", "javascript", "data", "vbscript", "shell"]
            .contains(&scheme.to_ascii_lowercase().as_str())
        && !url.contains(['\r', '\n'])
}

pub fn route<'a>(raw: &'a str, sources: &'a [Source]) -> (&'a str, Option<&'a str>) {
    let raw = raw.trim();
    let (prefix, query) = raw.split_once(char::is_whitespace).unwrap_or((raw, ""));
    sources
        .iter()
        .find(|s| s.enabled && s.keyword.eq_ignore_ascii_case(prefix))
        .map(|s| (query.trim(), Some(s.id.as_str())))
        .unwrap_or((raw, None))
}

pub fn eligible(raw: &str, sources: &[Source]) -> Vec<(Source, String)> {
    let (query, only) = route(raw, sources);
    if query.is_empty()
        || query.len() > 1024
        || (only.is_none()
            && (query.starts_with(['>', '@']) || crate::launcher::external_reserved(query)))
    {
        return Vec::new();
    }
    sources
        .iter()
        .filter(|s| s.enabled && only.map_or(s.global, |id| s.id == id))
        .map(|s| (s.clone(), query.into()))
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub enum Activation {
    Url(String),
    Callback {
        url: String,
        body: Value,
        pid: u32,
        session: String,
    },
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub action: Activation,
}

pub struct Runtime {
    pub generation: u64,
    pub query: String,
    pub hits: BTreeMap<String, Vec<Hit>>,
    pub pages: BTreeMap<String, PageState>,
    pub status: BTreeMap<String, String>,
    pub gates: HashMap<String, Arc<AtomicBool>>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            generation: 0,
            query: String::new(),
            hits: BTreeMap::new(),
            pages: BTreeMap::new(),
            status: BTreeMap::new(),
            gates: HashMap::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    pub page: usize,
    pub offset: usize,
    pub value: String,
}
impl Cursor {
    pub fn first(source: &Source) -> Self {
        Self {
            page: source.first_page,
            offset: 0,
            value: String::new(),
        }
    }
}
pub struct SearchPage {
    pub hits: Vec<Hit>,
    pub next: Option<Cursor>,
    pub total: Option<usize>,
}
pub struct PageState {
    pub next: Option<Cursor>,
    pub total: Option<usize>,
    pub loading: bool,
    pub error: Option<String>,
}
pub struct Permit(pub Arc<AtomicBool>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn at<'a>(value: &'a Value, path: &str) -> &'a Value {
    path.split('.')
        .filter(|p| !p.is_empty())
        .fold(value, |v, p| {
            if let Ok(i) = p.parse::<usize>() {
                &v[i]
            } else {
                &v[p]
            }
        })
}
fn text_at(value: &Value, path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    match at(value, path) {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn substitute(value: &mut Value, query: &str, limit: usize, cursor: &Cursor) {
    match value {
        Value::String(s) if s == "{limit}" => *value = json!(limit),
        Value::String(s) if s == "{page}" => *value = json!(cursor.page),
        Value::String(s) if s == "{offset}" => *value = json!(cursor.offset),
        Value::String(s) => {
            *s = s
                .replace("{page}", &cursor.page.to_string())
                .replace("{offset}", &cursor.offset.to_string())
                .replace("{cursor}", &cursor.value)
                .replace("{q}", query)
                .replace("{query}", query)
        }
        Value::Array(a) => a
            .iter_mut()
            .for_each(|v| substitute(v, query, limit, cursor)),
        Value::Object(o) => o
            .values_mut()
            .for_each(|v| substitute(v, query, limit, cursor)),
        _ => (),
    }
}
fn plain(s: &str, max: usize) -> String {
    let mut inside = false;
    let result: String = s
        .chars()
        .filter(|c| {
            if *c == '<' {
                inside = true;
                false
            } else if *c == '>' && inside {
                inside = false;
                false
            } else {
                !inside
            }
        })
        .take(max)
        .collect();
    result
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .replace(['\n', '\r'], " ")
}

fn request(source: &Source, url: &str, body: Option<&[u8]>, method: &str) -> Result<Value, String> {
    validate_url(url)?;
    let token = open_secret(&source.secret)?;
    let auth = if token.is_empty() {
        String::new()
    } else if source.auth_scheme.is_empty() {
        token
    } else {
        format!("{} {}", source.auth_scheme, token)
    };
    if auth.contains(['\r', '\n']) {
        return Err("认证内容无效".into());
    }
    let mut headers = vec![("Content-Type", "application/json")];
    if !auth.is_empty() {
        headers.push((source.auth_header.as_str(), auth.as_str()));
    }
    let req = http::Request {
        method,
        url,
        headers: &headers,
        body,
        receive_timeout_ms: source.timeout_ms,
    };
    let (status, raw) =
        http::fetch_search(&req, 2 * 1024 * 1024).map_err(|_| "连接失败或超时，软件可能未运行")?;
    if status != 200 {
        return Err(format!("接口返回 HTTP {status}"));
    }
    let value: Value = serde_json::from_slice(&raw).map_err(|_| "接口没有返回合法 JSON")?;
    if value["code"].as_i64().is_some_and(|code| code != 0) || value.get("error").is_some() {
        return Err("接口报告查询失败，请检查认证和请求配置".into());
    }
    Ok(value)
}

pub fn search(source: &Source, query: &str) -> Result<Vec<Hit>, String> {
    fetch_page(source, query, &Cursor::first(source)).map(|page| page.hits)
}

pub fn fetch_page(source: &Source, query: &str, cursor: &Cursor) -> Result<SearchPage, String> {
    validate(source, &[])?;
    let url = source
        .url
        .replace("{limit}", &source.limit.to_string())
        .replace("{page}", &cursor.page.to_string())
        .replace("{offset}", &cursor.offset.to_string())
        .replace("{cursor}", &encode(&cursor.value))
        .replace("{q}", &encode(query))
        .replace("{query}", &encode(query));
    let mut body: Value = serde_json::from_str(&source.request).unwrap_or(json!({}));
    substitute(&mut body, query, source.limit, cursor);
    let bytes = serde_json::to_vec(&body).map_err(|_| "请求正文无效")?;
    let data = request(
        source,
        &url,
        (source.method == "POST").then_some(bytes.as_slice()),
        &source.method,
    )?;
    let rows = at(&data, &source.results_path)
        .as_array()
        .ok_or("结果数组路径不正确，请检查字段映射")?;
    let local = http::parse_url(&source.url)
        .is_ok_and(|url| ["127.0.0.1", "localhost"].contains(&url.host.as_str()));
    let mut seen = std::collections::HashSet::new();
    let hits = rows
        .iter()
        .take(source.limit.min(50))
        .filter_map(|row| {
            let id = text_at(row, &source.id_path);
            let title = plain(&text_at(row, &source.title_path), 180);
            if id.is_empty() || title.is_empty() || !seen.insert(id.clone()) {
                return None;
            }
            let subtitle = plain(&text_at(row, &source.subtitle_path), 220);
            let snippet = plain(&text_at(row, &source.snippet_path), 250);
            let action = if !source.open_url.is_empty() {
                let url = source
                    .open_url
                    .replace("{id}", &encode(&id))
                    .replace("{query}", &encode(query));
                if !safe_open_url(&url) {
                    return None;
                }
                Activation::Url(url)
            } else {
                let payload = if row["action"].is_object() {
                    row["action"].clone()
                } else {
                    json!({"id":id,"session":data["session"]})
                };
                Activation::Callback {
                    url: source.activate_url.clone(),
                    body: payload,
                    pid: if local {
                        data["pid"]
                            .as_u64()
                            .and_then(|n| n.try_into().ok())
                            .unwrap_or(0)
                    } else {
                        0
                    },
                    session: text_at(&data, "session"),
                }
            };
            Some(Hit {
                id,
                title,
                subtitle: if snippet.is_empty() || snippet == subtitle {
                    subtitle
                } else if subtitle.is_empty() {
                    snippet
                } else {
                    format!("{subtitle} · {snippet}")
                },
                action,
            })
        })
        .collect();
    let consumed = rows.len().min(source.limit);
    let total = if source.total_path.is_empty() {
        None
    } else {
        at(&data, &source.total_path)
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
    };
    let next_value = text_at(&data, &source.next_cursor_path);
    let more = if source.pagination == "none" || consumed == 0 {
        false
    } else if source.pagination == "cursor" {
        !next_value.is_empty() && next_value != cursor.value
    } else if let Some(flag) = (!source.has_more_path.is_empty())
        .then(|| at(&data, &source.has_more_path).as_bool())
        .flatten()
    {
        flag
    } else if let Some(total) = total {
        cursor.offset.saturating_add(consumed) < total
    } else {
        consumed == source.limit
    };
    let next = more.then(|| Cursor {
        page: cursor.page.saturating_add(1),
        offset: cursor.offset.saturating_add(consumed),
        value: next_value,
    });
    Ok(SearchPage { hits, next, total })
}

pub fn activate(source: &Source, action: &Activation) -> Result<(), String> {
    if let Activation::Callback { url, body, .. } = action {
        if url != &source.activate_url {
            return Err("搜索源配置已更改，请重新搜索".into());
        }
        let bytes = serde_json::to_vec(body).map_err(|_| "激活参数无效")?;
        request(source, url, Some(&bytes), "POST")?;
    }
    Ok(())
}

/// Portable, provider-neutral configuration. Credentials are supplied separately on import.
pub fn portable(source: &Source) -> Value {
    let mut value = serde_json::to_value(source).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        object.remove("secret");
        object.remove("id");
    }
    value
}

pub fn parse_import(text: &str, existing: &[Source], encrypt: bool) -> Result<Vec<Source>, String> {
    if text.len() > 1024 * 1024 {
        return Err("配置文件不能超过 1 MB".into());
    }
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| "配置不是合法 JSON")?;
    let rows = if let Some(rows) = value.get("sources") {
        if value.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("配置版本须为 1".into());
        }
        rows.as_array().cloned().ok_or("sources 必须是数组")?
    } else {
        vec![value]
    };
    if rows.is_empty() || rows.len() > 50 {
        return Err("一次导入须包含 1～50 个搜索源".into());
    }
    let mut imported = Vec::new();
    let mut prefixes = std::collections::HashSet::new();
    for mut row in rows {
        let object = row.as_object_mut().ok_or("每个搜索源必须是 JSON 对象")?;
        if ["name", "keyword", "url"]
            .iter()
            .any(|key| !object.get(*key).is_some_and(Value::is_string))
        {
            return Err("搜索源需要 name、keyword、url 字符串字段".into());
        }
        if object.contains_key("secret") || object.contains_key("discovery_file") {
            return Err("请使用 token 字段和完整 HTTP 地址，不要导入本机加密字段或发现文件".into());
        }
        let token = object.remove("token");
        if token.as_ref().is_some_and(|value| !value.is_string()) {
            return Err("token 字段必须是字符串".into());
        }
        object.remove("id");
        let mut source: Source = serde_json::from_value(row).map_err(|_| "配置字段类型错误")?;
        if !prefixes.insert(source.keyword.to_ascii_lowercase()) {
            return Err("导入配置包含重复的搜索前缀".into());
        }
        let old = existing
            .iter()
            .find(|s| s.keyword.eq_ignore_ascii_case(&source.keyword));
        if let Some(old) = old {
            source.id = old.id.clone();
            source.secret = old.secret.clone();
        }
        if let Some(token) = token {
            if encrypt {
                source.secret = seal_secret(token.as_str().unwrap())?;
            }
        }
        validate(&source, existing)?;
        imported.push(source);
    }
    Ok(imported)
}

pub fn import_schema() -> Value {
    let mut properties = serde_json::Map::new();
    let fields = [
        ("name", "显示名称"),
        ("keyword", "搜索前缀：英文字母、数字或连字符"),
        (
            "url",
            "完整 HTTP(S) 搜索地址；GET 可含 {query}、{limit}、{page}、{offset}、{cursor}",
        ),
        ("method", "GET 或 POST"),
        (
            "request",
            "JSON 正文字符串；占位符 {query}、{limit}、{page}、{offset}、{cursor}；数值占位符用 JSON 字符串包裹",
        ),
        ("results_path", "结果数组的点分路径；根数组留空"),
        ("id_path", "每条结果的稳定 ID 字段路径"),
        ("title_path", "标题字段路径"),
        ("subtitle_path", "副标题字段路径，可留空"),
        ("snippet_path", "摘要字段路径，可留空"),
        (
            "open_url",
            "打开链接模板，如 notes://open/{id}；与激活接口二选一",
        ),
        (
            "activate_url",
            "完整 HTTP(S) 激活地址；POST 结果 action 对象或 {id,session}",
        ),
        ("auth_header", "认证请求头，默认 Authorization"),
        ("auth_scheme", "认证前缀，如 Bearer、Token 或留空"),
        (
            "token",
            "可选认证密钥；省略时保留已有凭据，空字符串清除；导入后仅在本机加密保存",
        ),
        ("pagination", "none / page / offset / cursor"),
        ("total_path", "总数的字段路径，可留空"),
        ("has_more_path", "是否有下一页的布尔字段路径，可留空"),
        ("next_cursor_path", "下一页游标的字段路径；cursor 分页必填"),
    ];
    for (name, description) in fields {
        properties.insert(
            name.into(),
            json!({"type":"string","description":description}),
        );
    }
    properties.insert(
        "method".into(),
        json!({"type":"string","enum":["GET","POST"]}),
    );
    properties.insert(
        "pagination".into(),
        json!({"type":"string","enum":["none","page","offset","cursor"]}),
    );
    for key in ["enabled", "global"] {
        properties.insert(key.into(), json!({"type":"boolean","default":true}));
    }
    properties.insert(
        "first_page".into(),
        json!({"type":"integer","minimum":0,"maximum":1,"default":1}),
    );
    properties.insert(
        "limit".into(),
        json!({"type":"integer","minimum":1,"maximum":50,"default":10}),
    );
    properties.insert(
        "timeout_ms".into(),
        json!({"type":"integer","minimum":200,"maximum":10000,"default":2000}),
    );
    let mut example = portable(&Source::default());
    example["pagination"] = json!("page");
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "title":"LovelyLauncher HTTP search configuration", "type":"object",
        "required":["version","sources"], "properties":{"version":{"const":1},"sources":{"type":"array","minItems":1,"maxItems":50,
        "items":{"type":"object","required":["name","keyword","url"],"properties":properties,"additionalProperties":false}}},
        "additionalProperties":false,"examples":[{"version":1,"sources":[example]}]})
}

pub fn seal_secret(text: &str) -> Result<String, String> {
    crypt(text.as_bytes(), true)
}
pub fn open_secret(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Ok(String::new());
    }
    let hex = text
        .strip_prefix("dpapi:")
        .ok_or("认证信息格式无效，请重新填写 Token")?;
    if hex.len() % 2 != 0 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("认证信息无效".into());
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "认证信息无效")?;
    crypt(&bytes, false)
}
fn crypt(bytes: &[u8], protect: bool) -> Result<String, String> {
    use windows::Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    if bytes.is_empty() {
        return Ok(String::new());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        if protect {
            CryptProtectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .map_err(|_| "无法访问本机认证信息，请重新填写 Token")?;
        let raw = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let value = if protect {
            Ok(format!(
                "dpapi:{}",
                raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
            ))
        } else {
            String::from_utf8(raw.to_vec()).map_err(|_| "认证信息无效".to_string())
        };
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    #[test]
    fn json_substitution_does_not_inject_query() {
        let mut v = json!({"query":"{query}","pageSize":"{limit}"});
        substitute(
            &mut v,
            "a\"\\\n中文",
            12,
            &Cursor::first(&Source::default()),
        );
        assert_eq!(v["query"], "a\"\\\n中文");
        assert_eq!(v["pageSize"], 12);
        assert!(serde_json::from_str::<Value>(&v.to_string()).is_ok());
    }
    #[test]
    fn prefixes_and_reserved_queries() {
        let s = vec![
            Source {
                id: "documents".into(),
                keyword: "docs".into(),
                ..Default::default()
            },
            Source {
                id: "notes".into(),
                keyword: "notes".into(),
                ..Default::default()
            },
        ];
        assert_eq!(route("notes search", &s), ("search", Some("notes")));
        assert_eq!(eligible("docs", &s).len(), 0);
        assert_eq!(eligible("@ai secret", &s).len(), 0);
        assert_eq!(eligible("cb secret", &s).len(), 0);
        assert_eq!(eligible("SM2", &s).len(), 2);
    }
    #[test]
    fn validate_limits_and_auth() {
        let mut s = Source::default();
        assert!(validate(&s, &[]).is_ok());
        s.auth_header = "Auth\r\nInjected".into();
        assert!(validate(&s, &[]).is_err());
        s.auth_header = "Authorization".into();
        s.url = "http://example.com/search".into();
        assert!(validate(&s, &[]).is_err());
    }
    #[test]
    fn field_mapping_and_text() {
        assert_eq!(
            text_at(&json!({"data":{"items":[{"id":7}]}}), "data.items.0.id"),
            "7"
        );
        assert_eq!(plain("<mark>SM2</mark> &amp; 算法", 100), "SM2 & 算法");
    }
    #[test]
    fn malformed_credential_is_an_error() {
        assert!(open_secret("dpapi:中a").is_err());
        assert!(open_secret("plaintext-token").is_err());
    }
    #[test]
    fn import_is_generic_and_updates_by_prefix() {
        let original = Source {
            id: "existing-id".into(),
            name: "Original".into(),
            keyword: "docs".into(),
            secret: "dpapi:abcd".into(),
            ..Default::default()
        };
        let mut document = portable(&original);
        document["name"] = json!("Updated");
        assert!(document.get("secret").is_none());
        assert!(document.get("id").is_none());
        let imported = parse_import(
            &json!({"version":1,"sources":[document]}).to_string(),
            &[original],
            false,
        )
        .unwrap();
        assert_eq!(imported[0].id, "existing-id");
        assert_eq!(imported[0].secret, "dpapi:abcd");
        assert_eq!(imported[0].name, "Updated");
        assert!(parse_import(r#"{"version":2,"sources":[]}"#, &[], false).is_err());
        assert!(
            parse_import(
                r#"{"name":"bad","keyword":"bad","url":"/search","discovery_file":"anything"}"#,
                &[],
                false
            )
            .is_err()
        );
        let template = import_schema();
        assert!(parse_import(&template["examples"][0].to_string(), &[], false).is_ok());
    }
    #[test]
    fn duplicate_imports_and_missing_pagination_placeholder_are_errors() {
        let source = portable(&Source::default());
        assert!(
            parse_import(
                &json!({"version":1,"sources":[source.clone(),source]}).to_string(),
                &[],
                false
            )
            .is_err()
        );
        let mut source = Source {
            pagination: "page".into(),
            request: r#"{"query":"{query}"}"#.into(),
            ..Default::default()
        };
        assert!(validate(&source, &[]).is_err());
        source.pagination = "cursor".into();
        source.next_cursor_path.clear();
        assert!(validate(&source, &[]).is_err());
    }
    #[test]
    #[ignore = "requires the current user's Windows DPAPI"]
    fn imported_tokens_are_protected_and_not_exported() {
        let mut document = portable(&Source::default());
        document["token"] = json!("private-import-token");
        let sources = parse_import(&document.to_string(), &[], true).unwrap();
        assert_eq!(
            open_secret(&sources[0].secret).unwrap(),
            "private-import-token"
        );
        assert!(
            !serde_json::to_string(&sources)
                .unwrap()
                .contains("private-import-token")
        );
        assert!(portable(&sources[0]).get("token").is_none());
    }
    fn paginated_server(responses: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/search?q={{query}}&page={{page}}&offset={{offset}}&cursor={{cursor}}",
            listener.local_addr().unwrap()
        );
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
                requests.push(String::from_utf8_lossy(&request).into_owned());
                let body = body.to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });
        (url, handle)
    }
    #[test]
    #[ignore = "requires loopback networking"]
    fn pages_are_appended_until_the_reported_total() {
        let (url, server) = paginated_server(
            (1..=3)
                .map(|id| json!({"results":[{"id":id,"title":format!("result {id}")}],"total":3}))
                .collect(),
        );
        let source = Source {
            url,
            method: "GET".into(),
            pagination: "page".into(),
            limit: 1,
            ..Default::default()
        };
        let first = fetch_page(&source, "query", &Cursor::first(&source)).unwrap();
        assert_eq!(first.total, Some(3));
        assert_eq!(first.hits[0].id, "1");
        let second = fetch_page(&source, "query", &first.next.unwrap()).unwrap();
        assert_eq!(second.hits[0].id, "2");
        let third = fetch_page(&source, "query", &second.next.unwrap()).unwrap();
        assert_eq!(third.hits[0].id, "3");
        assert!(third.next.is_none());
        let requests = server.join().unwrap();
        assert!(requests[0].contains("page=1&offset=0"));
        assert!(requests[1].contains("page=2&offset=1"));
    }
    #[test]
    #[ignore = "requires loopback networking"]
    fn offset_and_cursor_pagination_stop_correctly() {
        let (url, server) = paginated_server(vec![
            json!({"results":[{"id":1,"title":"one"},{"id":2,"title":"two"}],"has_more":true}),
            json!({"results":[{"id":3,"title":"three"}],"has_more":false}),
        ]);
        let source = Source {
            url,
            method: "GET".into(),
            pagination: "offset".into(),
            limit: 2,
            ..Default::default()
        };
        let first = fetch_page(&source, "query", &Cursor::first(&source)).unwrap();
        let last = fetch_page(&source, "query", &first.next.unwrap()).unwrap();
        assert!(last.next.is_none());
        assert!(server.join().unwrap()[1].contains("offset=2"));
        let (url, server) = paginated_server(vec![
            json!({"results":[{"id":1,"title":"one"}],"next_cursor":"opaque +/"}),
            json!({"results":[{"id":2,"title":"two"}],"next_cursor":"opaque +/"}),
        ]);
        let source = Source {
            url,
            method: "GET".into(),
            pagination: "cursor".into(),
            limit: 1,
            ..Default::default()
        };
        let first = fetch_page(&source, "query", &Cursor::first(&source)).unwrap();
        let last = fetch_page(&source, "query", &first.next.unwrap()).unwrap();
        assert!(last.next.is_none());
        assert!(server.join().unwrap()[1].contains("cursor=opaque%20%2B%2F"));
    }
    fn test_server(
        status: u32,
        extra: &str,
        body: &str,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/search?q={{query}}&limit={{limit}}",
            listener.local_addr().unwrap()
        );
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
            body.len()
        );
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            while !request.windows(4).any(|s| s == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (url, thread)
    }
    #[test]
    #[ignore = "requires loopback networking"]
    fn custom_get_mapping_and_url_encoding() {
        let (url, server) = test_server(
            200,
            "",
            r#"{"payload":{"rows":[{"uid":"abc /中文","label":"<mark>SM2</mark>","path":"算法","summary":"签名验签"},{"uid":"abc /中文","label":"duplicate"}]}}"#,
        );
        let source = Source {
            url,
            method: "GET".into(),
            results_path: "payload.rows".into(),
            id_path: "uid".into(),
            title_path: "label".into(),
            subtitle_path: "path".into(),
            snippet_path: "summary".into(),
            open_url: "exampleapp://open/{id}".into(),
            activate_url: String::new(),
            ..Default::default()
        };
        let hits = search(&source, "a &中文").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "SM2");
        assert_eq!(
            hits[0].action,
            Activation::Url("exampleapp://open/abc%20%2F%E4%B8%AD%E6%96%87".into())
        );
        assert!(
            server
                .join()
                .unwrap()
                .contains("q=a%20%26%E4%B8%AD%E6%96%87&limit=10")
        );
    }
    #[test]
    #[ignore = "requires loopback networking"]
    fn redirects_are_not_followed() {
        let (url, server) = test_server(302, "Location: http://127.0.0.1:1/redirected\r\n", "{}");
        let source = Source {
            url,
            method: "GET".into(),
            ..Default::default()
        };
        assert_eq!(search(&source, "test").unwrap_err(), "接口返回 HTTP 302");
        server.join().unwrap();
    }
    #[test]
    #[ignore = "requires the current user's Windows DPAPI"]
    fn credentials_roundtrip() {
        let sealed = seal_secret("test-key").unwrap();
        assert!(!sealed.contains("test-key"));
        assert_eq!(open_secret(&sealed).unwrap(), "test-key");
    }
    #[test]
    #[ignore = "requires configured running local applications"]
    fn live_sources() {
        let config = Config::load();
        for source in config
            .settings
            .external_sources
            .iter()
            .filter(|s| s.enabled)
        {
            let hits = search(source, "SM2").unwrap();
            println!("{}: {} results", source.name, hits.len());
        }
    }
}
