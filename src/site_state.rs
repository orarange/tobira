use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use crate::url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StorageKind {
    Local,
    Session,
}

#[derive(Debug, Default)]
struct SiteState {
    storages: BTreeMap<(String, StorageKind), StorageBucket>,
    cookies: Vec<CookieEntry>,
}

#[derive(Debug, Default)]
struct StorageBucket {
    order: Vec<String>,
    values: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct CookieEntry {
    name: String,
    value: String,
    domain: String,
    host_only: bool,
    path: String,
    secure: bool,
    http_only: bool,
}

static SITE_STATE: OnceLock<Mutex<SiteState>> = OnceLock::new();

fn global_state() -> &'static Mutex<SiteState> {
    SITE_STATE.get_or_init(|| Mutex::new(SiteState::default()))
}

pub fn storage_get_item(kind: StorageKind, url: &Url, key: &str) -> Option<String> {
    let state = global_state().lock().ok()?;
    state
        .storages
        .get(&(url.origin(), kind))
        .and_then(|bucket| bucket.values.get(key).cloned())
}

pub fn storage_set_item(kind: StorageKind, url: &Url, key: String, value: String) {
    let mut state = global_state().lock().unwrap();
    let bucket = state.storages.entry((url.origin(), kind)).or_default();
    if !bucket.values.contains_key(&key) {
        bucket.order.push(key.clone());
    }
    bucket.values.insert(key, value);
}

pub fn storage_remove_item(kind: StorageKind, url: &Url, key: &str) {
    let mut state = global_state().lock().unwrap();
    let Some(bucket) = state.storages.get_mut(&(url.origin(), kind)) else {
        return;
    };
    if bucket.values.remove(key).is_some() {
        bucket.order.retain(|existing| existing != key);
    }
}

pub fn storage_clear(kind: StorageKind, url: &Url) {
    let mut state = global_state().lock().unwrap();
    state.storages.remove(&(url.origin(), kind));
}

pub fn storage_length(kind: StorageKind, url: &Url) -> usize {
    let state = global_state().lock().unwrap();
    state
        .storages
        .get(&(url.origin(), kind))
        .map(|bucket| bucket.values.len())
        .unwrap_or(0)
}

pub fn storage_key(kind: StorageKind, url: &Url, index: usize) -> Option<String> {
    let state = global_state().lock().ok()?;
    state
        .storages
        .get(&(url.origin(), kind))
        .and_then(|bucket| bucket.order.get(index).cloned())
}

pub fn document_cookie_get(url: &Url) -> String {
    cookie_pairs_for_url(url, false)
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn document_cookie_set(url: &Url, cookie_line: &str) {
    let Some(update) = parse_cookie_line(url, cookie_line, false) else {
        return;
    };
    apply_cookie_update(update);
}

pub fn cookie_header_for_url(url: &Url) -> Option<String> {
    let pairs = cookie_pairs_for_url(url, true);
    (!pairs.is_empty()).then(|| {
        pairs
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

pub fn apply_response_set_cookie_headers(url: &Url, headers: &[String]) {
    for header in headers {
        if let Some(update) = parse_cookie_line(url, header, true) {
            apply_cookie_update(update);
        }
    }
}

#[derive(Debug, Clone)]
struct CookieUpdate {
    delete: bool,
    entry: CookieEntry,
}

fn apply_cookie_update(update: CookieUpdate) {
    let mut state = global_state().lock().unwrap();
    state
        .cookies
        .retain(|existing| !existing.same_identity(&update.entry));
    if !update.delete {
        state.cookies.push(update.entry);
    }
}

fn parse_cookie_line(url: &Url, cookie_line: &str, allow_http_only: bool) -> Option<CookieUpdate> {
    let mut parts = cookie_line.split(';');
    let first = parts.next()?.trim();
    let (name, value) = first.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let default_path = default_cookie_path(url);
    let mut domain = url.host.to_ascii_lowercase();
    let mut host_only = true;
    let mut path = default_path.clone();
    let mut secure = false;
    let mut http_only = false;
    let mut delete = false;

    for attribute in parts {
        let attribute = attribute.trim();
        if attribute.is_empty() {
            continue;
        }

        if let Some((key, value)) = attribute.split_once('=') {
            match key.trim().to_ascii_lowercase().as_str() {
                "domain" => {
                    let normalized = normalize_cookie_domain(value);
                    if normalized.is_empty() || !cookie_domain_matches(url, &normalized) {
                        return None;
                    }
                    domain = normalized;
                    host_only = false;
                }
                "path" => {
                    path = normalize_cookie_path(value, &default_path);
                }
                "max-age" => {
                    if value.trim().parse::<i64>().ok().is_some_and(|age| age <= 0) {
                        delete = true;
                    }
                }
                "expires" => {
                    if value.to_ascii_lowercase().contains("1970") {
                        delete = true;
                    }
                }
                _ => {}
            }
        } else {
            match attribute.to_ascii_lowercase().as_str() {
                "secure" => secure = true,
                "httponly" if allow_http_only => http_only = true,
                _ => {}
            }
        }
    }

    Some(CookieUpdate {
        delete,
        entry: CookieEntry {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain,
            host_only,
            path,
            secure,
            http_only,
        },
    })
}

fn cookie_pairs_for_url(url: &Url, include_http_only: bool) -> Vec<(String, String)> {
    let state = global_state().lock().unwrap();
    state
        .cookies
        .iter()
        .filter(|cookie| cookie_matches_url(cookie, url))
        .filter(|cookie| include_http_only || !cookie.http_only)
        .map(|cookie| (cookie.name.clone(), cookie.value.clone()))
        .collect()
}

fn cookie_matches_url(cookie: &CookieEntry, url: &Url) -> bool {
    if cookie.secure && url.scheme != "https" {
        return false;
    }

    let host = url.host.to_ascii_lowercase();
    let path = cookie_path_from_url(url);
    if !cookie_domain_matches_host(&host, cookie) {
        return false;
    }

    cookie_path_matches(&path, &cookie.path)
}

fn cookie_domain_matches_host(host: &str, cookie: &CookieEntry) -> bool {
    if cookie.host_only {
        return host.eq_ignore_ascii_case(&cookie.domain);
    }

    host.eq_ignore_ascii_case(&cookie.domain)
        || host.strip_suffix(&format!(".{}", cookie.domain)).is_some()
}

fn cookie_domain_matches(url: &Url, cookie_domain: &str) -> bool {
    let host = url.host.to_ascii_lowercase();
    host.eq_ignore_ascii_case(cookie_domain)
        || host.strip_suffix(&format!(".{cookie_domain}")).is_some()
}

fn cookie_path_matches(path: &str, cookie_path: &str) -> bool {
    if cookie_path == "/" {
        return true;
    }

    if path == cookie_path {
        return true;
    }

    if let Some(remainder) = path.strip_prefix(cookie_path) {
        return cookie_path.ends_with('/') || remainder.starts_with('/');
    }

    false
}

fn cookie_path_from_url(url: &Url) -> String {
    url.path
        .split(['?', '#'])
        .next()
        .map(default_cookie_path_from_path)
        .unwrap_or_else(|| "/".to_string())
}

fn default_cookie_path(url: &Url) -> String {
    default_cookie_path_from_path(
        url.path
            .split(['?', '#'])
            .next()
            .unwrap_or(url.path.as_str()),
    )
}

fn default_cookie_path_from_path(path: &str) -> String {
    if path.is_empty() || !path.starts_with('/') {
        return "/".to_string();
    }

    if path == "/" {
        return "/".to_string();
    }

    if let Some((prefix, _)) = path.rsplit_once('/') {
        if prefix.is_empty() {
            "/".to_string()
        } else {
            format!("{prefix}/")
        }
    } else {
        "/".to_string()
    }
}

fn normalize_cookie_domain(domain: &str) -> String {
    domain.trim().trim_start_matches('.').to_ascii_lowercase()
}

fn normalize_cookie_path(path: &str, default_path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return default_path.to_string();
    }
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

impl CookieEntry {
    fn same_identity(&self, other: &Self) -> bool {
        self.name == other.name
            && self.domain.eq_ignore_ascii_case(&other.domain)
            && self.host_only == other.host_only
            && self.path == other.path
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StorageKind, apply_response_set_cookie_headers, cookie_header_for_url, document_cookie_get,
        document_cookie_set, storage_clear, storage_get_item, storage_key, storage_length,
        storage_remove_item, storage_set_item,
    };
    use crate::url::Url;

    #[test]
    fn storage_persists_per_origin() {
        let origin = Url::parse("https://storage.example/app").unwrap();
        let other_origin = Url::parse("https://other.example/app").unwrap();

        storage_clear(StorageKind::Local, &origin);
        storage_set_item(
            StorageKind::Local,
            &origin,
            "theme".to_string(),
            "dark".to_string(),
        );

        assert_eq!(
            storage_get_item(StorageKind::Local, &origin, "theme"),
            Some("dark".to_string())
        );
        assert_eq!(storage_length(StorageKind::Local, &origin), 1);
        assert_eq!(
            storage_key(StorageKind::Local, &origin, 0),
            Some("theme".to_string())
        );
        assert_eq!(
            storage_get_item(StorageKind::Local, &other_origin, "theme"),
            None
        );

        storage_remove_item(StorageKind::Local, &origin, "theme");
        assert_eq!(storage_length(StorageKind::Local, &origin), 0);
    }

    #[test]
    fn cookies_round_trip_by_path_and_response_headers() {
        let url = Url::parse("https://cookie.example/docs/page").unwrap();
        let other_url = Url::parse("https://cookie.example/other").unwrap();

        document_cookie_set(&url, "session=abc123; path=/docs");
        document_cookie_set(&url, "theme=dark");
        apply_response_set_cookie_headers(
            &url,
            &[String::from("httpOnly=1; path=/docs; HttpOnly")],
        );

        let cookie_string = document_cookie_get(&url);
        assert!(cookie_string.contains("session=abc123"));
        assert!(cookie_string.contains("theme=dark"));
        assert!(!cookie_string.contains("httpOnly=1"));
        assert!(
            cookie_header_for_url(&url)
                .expect("cookie header")
                .contains("httpOnly=1")
        );
        assert!(!document_cookie_get(&other_url).contains("session=abc123"));
    }
}
