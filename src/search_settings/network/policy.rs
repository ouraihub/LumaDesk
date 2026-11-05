//! Validation and blacklist policy shared by search and fetch transport.
use super::*;
pub(in crate::search_settings) fn validate_url(raw: &str) -> Result<Url, String> {
    if raw.len() > 4096 {
        return Err("URL 不得超过 4096 字节".into());
    }
    let url = search::validate_search_url(raw)
        .map_err(|_| "URL 必须为 HTTPS 或回环 HTTP，且不能包含用户名/密码".to_owned())?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err("URL 不得包含查询参数或片段".into());
    }
    Ok(url)
}

pub(in crate::search_settings) fn preferences_from_draft(
    prefs: &SearchPreferences,
    text: &str,
) -> Result<SearchPreferences, String> {
    if !(1..=50).contains(&prefs.max_results) || !(100..=20_000).contains(&prefs.cutoff) {
        return Err("结果数须为 1–50，截断长度须为 100–20000".into());
    }
    let rules = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_rule)
        .collect::<Result<Vec<_>, _>>()?;
    if rules.len() > 32 {
        return Err("黑名单最多 32 条".into());
    }
    let mut next = prefs.clone();
    next.blacklist = rules;
    Ok(next)
}

pub(in crate::search_settings) fn normalize_rule(raw: &str) -> Result<String, String> {
    let invalid = || {
        "黑名单规则无效：仅支持域名、域名通配和完整 HTTP(S) URL 通配；不能含空白/凭据/片段"
            .to_owned()
    };
    if raw.chars().count() > 256
        || raw.is_empty()
        || !raw.is_ascii()
        || raw.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(invalid());
    }
    if let Some((scheme, rest)) = raw.split_once("://") {
        if !matches!(scheme, "http" | "https" | "*")
            || rest.contains('@')
            || rest.contains('#')
            || rest.contains('\\')
        {
            return Err(invalid());
        }
        let (authority, path) = rest.split_once('/').ok_or_else(invalid)?;
        let host = authority
            .split_once(':')
            .map_or(authority, |(host, _)| host);
        validate_host_pattern(host)?;
        if let Some((_, port)) = authority.split_once(':')
            && port.parse::<u16>().ok().filter(|p| *p != 0).is_none()
        {
            return Err(invalid());
        }
        if path.contains(':') || path.contains('"') || path.contains('<') || path.contains('>') {
            return Err(invalid());
        }
        Ok(format!(
            "{scheme}://{}/{}",
            authority.to_ascii_lowercase(),
            path
        ))
    } else {
        validate_host_pattern(raw)?;
        Ok(raw.to_ascii_lowercase())
    }
}

fn validate_host_pattern(host: &str) -> Result<(), String> {
    if host.is_empty()
        || host.len() > 253
        || (!host.contains('.') && !host.eq_ignore_ascii_case("localhost"))
    {
        return Err("无效黑名单域名".into());
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'*' | b'?'))
        {
            return Err("无效黑名单域名通配".into());
        }
    }
    Ok(())
}

fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < t.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            retry += 1;
            j = retry;
            i = s + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == b'*' {
        i += 1;
    }
    i == p.len()
}

pub(in crate::search_settings) fn blocked(url: &Url, rules: &[String]) -> bool {
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    rules.iter().any(|rule| {
        if rule.contains("://") {
            glob(rule, url.as_str())
        } else if rule.contains(['*', '?']) {
            glob(rule, &host)
        } else {
            host == *rule
                || host
                    .strip_suffix(rule.as_str())
                    .is_some_and(|s| s.ends_with('.'))
        }
    })
}
