use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use brotli::Decompressor;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use rustls_platform_verifier::ConfigVerifierExt;

use crate::error::{BrowserError, Result};
use crate::site_state;
use crate::url::Url;

const MAX_REDIRECTS: usize = 5;
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36 Tobira/0.1";
const RESPONSE_HEADER_SLACK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub final_url: Url,
    pub status_code: u16,
    pub reason_phrase: String,
    pub headers: HashMap<String, String>,
    pub set_cookie_headers: Vec<String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        let key = name.to_ascii_lowercase();
        self.headers.get(&key).map(String::as_str)
    }
}

#[derive(Debug, Clone, Default)]
pub struct HttpRequestOptions {
    pub method: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Vec<u8>>,
}

pub fn fetch(url: &Url) -> Result<HttpResponse> {
    fetch_inner(url, 0, None, None, HttpRequestOptions::default())
}

pub fn fetch_with_limits(url: &Url, max_body_bytes: usize) -> Result<HttpResponse> {
    fetch_inner(
        url,
        0,
        Some(max_body_bytes),
        None,
        HttpRequestOptions::default(),
    )
}

pub fn fetch_with_limits_same_origin(
    url: &Url,
    max_body_bytes: usize,
    origin: &Url,
) -> Result<HttpResponse> {
    fetch_inner(
        url,
        0,
        Some(max_body_bytes),
        Some(origin),
        HttpRequestOptions::default(),
    )
}

pub fn fetch_with_request_with_limits_same_origin(
    url: &Url,
    max_body_bytes: usize,
    origin: &Url,
    request: &HttpRequestOptions,
) -> Result<HttpResponse> {
    fetch_inner(url, 0, Some(max_body_bytes), Some(origin), request.clone())
}

fn fetch_inner(
    url: &Url,
    redirect_count: usize,
    max_body_bytes: Option<usize>,
    same_origin: Option<&Url>,
    request: HttpRequestOptions,
) -> Result<HttpResponse> {
    if redirect_count > MAX_REDIRECTS {
        return Err(BrowserError::message("too many redirects"));
    }

    let address = format!("{}:{}", url.host, url.port);
    let tcp_stream = TcpStream::connect(address)?;
    tcp_stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    tcp_stream.set_write_timeout(Some(Duration::from_secs(20)))?;
    let mut stream = open_stream(url, tcp_stream)?;
    let request_bytes = build_request_bytes(url, &request);
    stream.write_all(&request_bytes)?;

    let response_bytes = read_response_bytes(
        &mut stream,
        max_body_bytes.map(|limit| limit.saturating_add(RESPONSE_HEADER_SLACK_BYTES)),
    )?;

    let response = parse_response_with_limits(url, &response_bytes, max_body_bytes)?;
    site_state::apply_response_set_cookie_headers(url, &response.set_cookie_headers);

    if is_redirect(response.status_code) {
        if let Some(location) = response.header("location") {
            let next_url = url.resolve(location)?;
            if let Some(origin) = same_origin
                && !origin.shares_origin(&next_url)
            {
                return Err(BrowserError::message(
                    "cross-origin redirect target is blocked",
                ));
            }
            let next_request = redirect_followup_request(&request, response.status_code);
            return fetch_inner(
                &next_url,
                redirect_count + 1,
                max_body_bytes,
                same_origin,
                next_request,
            );
        }
    }

    Ok(response)
}

#[cfg(test)]
fn parse_response(url: &Url, bytes: &[u8]) -> Result<HttpResponse> {
    parse_response_with_limits(url, bytes, None)
}

fn parse_response_with_limits(
    url: &Url,
    bytes: &[u8],
    max_body_bytes: Option<usize>,
) -> Result<HttpResponse> {
    let Some(header_end) = find_bytes(bytes, b"\r\n\r\n") else {
        return Err(BrowserError::message(
            "invalid HTTP response: missing header separator",
        ));
    };

    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let body_bytes = &bytes[header_end + 4..];
    let mut lines = header_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| BrowserError::message("invalid HTTP response: missing status line"))?;
    let mut status_parts = status_line.splitn(3, ' ');
    let _version = status_parts
        .next()
        .ok_or_else(|| BrowserError::message("invalid HTTP response: missing version"))?;
    let status_code = status_parts
        .next()
        .ok_or_else(|| BrowserError::message("invalid HTTP response: missing status code"))?
        .parse::<u16>()
        .map_err(|_| BrowserError::message("invalid HTTP response: bad status code"))?;
    let reason_phrase = status_parts.next().unwrap_or("").to_string();

    let mut headers = HashMap::new();
    let mut set_cookie_headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "set-cookie" {
                set_cookie_headers.push(value.clone());
            }
            headers.insert(name, value);
        }
    }

    let body = match headers.get("transfer-encoding") {
        Some(value) if value.to_ascii_lowercase().contains("chunked") => {
            decode_chunked(body_bytes, max_body_bytes)?
        }
        _ => body_bytes.to_vec(),
    };
    if let Some(limit) = max_body_bytes
        && body.len() > limit
    {
        return Err(BrowserError::message(format!(
            "response body exceeded limit of {limit} bytes"
        )));
    }
    let body = decode_content(body, headers.get("content-encoding"), max_body_bytes)?;

    Ok(HttpResponse {
        final_url: url.clone(),
        status_code,
        reason_phrase,
        headers,
        set_cookie_headers,
        body,
    })
}

fn is_redirect(status_code: u16) -> bool {
    matches!(status_code, 301 | 302 | 303 | 307 | 308)
}

fn redirect_followup_request(request: &HttpRequestOptions, status_code: u16) -> HttpRequestOptions {
    let mut next = request.clone();
    let method = next.method.trim().to_ascii_uppercase();
    if should_switch_to_get_after_redirect(status_code, &method) {
        next.method = "GET".to_string();
        next.body = None;
    }
    next
}

fn should_switch_to_get_after_redirect(status_code: u16, method: &str) -> bool {
    matches!(status_code, 303)
        || matches!(status_code, 301 | 302) && !matches!(method, "GET" | "HEAD")
}

fn build_request_bytes(url: &Url, request: &HttpRequestOptions) -> Vec<u8> {
    let method = normalized_request_method(&request.method);
    let cookie_header = site_state::cookie_header_for_url(url)
        .map(|value| format!("Cookie: {value}\r\n"))
        .unwrap_or_default();
    let mut header_lines = String::new();
    for (name, value) in &request.headers {
        if name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        header_lines.push_str(&format!("{name}: {value}\r\n"));
    }

    let body = request.body.as_deref().unwrap_or(&[]);
    let body_length = body.len();
    let content_length = if matches!(method.as_str(), "GET" | "HEAD") && body_length == 0 {
        None
    } else {
        Some(body_length)
    };
    let content_length_line = content_length
        .map(|len| format!("Content-Length: {len}\r\n"))
        .unwrap_or_default();

    let request_text = format!(
        "{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/png,*/*;q=0.8\r\nAccept-Language: ja-JP,ja;q=0.9,en-US;q=0.8,en;q=0.7\r\nAccept-Encoding: gzip, deflate, br\r\nCache-Control: no-cache\r\nPragma: no-cache\r\nConnection: close\r\nUpgrade-Insecure-Requests: 1\r\nSec-CH-UA: \"Chromium\";v=\"136\", \"Google Chrome\";v=\"136\", \"Not/A)Brand\";v=\"99\"\r\nSec-CH-UA-Mobile: ?0\r\nSec-CH-UA-Platform: \"Windows\"\r\nSec-Fetch-Dest: document\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Site: none\r\nSec-Fetch-User: ?1\r\n{header_lines}{content_length_line}{cookie_header}\r\n",
        url.path,
        url.host_header(),
        USER_AGENT
    );
    let mut request_bytes = request_text.into_bytes();
    if !body.is_empty() {
        request_bytes.extend_from_slice(body);
    }
    request_bytes
}

fn normalized_request_method(method: &str) -> String {
    let trimmed = method.trim();
    if trimmed.is_empty() {
        "GET".to_string()
    } else {
        trimmed.to_ascii_uppercase()
    }
}

fn open_stream(url: &Url, tcp_stream: TcpStream) -> Result<Box<dyn ReadWrite>> {
    match url.scheme.as_str() {
        "http" => Ok(Box::new(tcp_stream)),
        "https" => {
            let config = ClientConfig::with_platform_verifier()
                .map_err(|error| BrowserError::message(error.to_string()))?;
            let server_name = ServerName::try_from(url.host.clone())
                .map_err(|_| BrowserError::message("invalid https host name"))?;
            let connection = ClientConnection::new(Arc::new(config), server_name)
                .map_err(|error| BrowserError::message(error.to_string()))?;
            Ok(Box::new(StreamOwned::new(connection, tcp_stream)))
        }
        _ => Err(BrowserError::message(format!(
            "unsupported scheme: {}",
            url.scheme
        ))),
    }
}

fn decode_content(
    body: Vec<u8>,
    content_encoding: Option<&String>,
    max_output_bytes: Option<usize>,
) -> Result<Vec<u8>> {
    let Some(encoding) = content_encoding else {
        return Ok(body);
    };

    let encoding = encoding.to_ascii_lowercase();
    let primary = encoding.split(',').next().unwrap_or("").trim();

    match primary {
        "" | "identity" => Ok(body),
        "gzip" => read_all(GzDecoder::new(Cursor::new(body)), max_output_bytes),
        "deflate" => decode_deflate(body, max_output_bytes),
        "br" => read_all(Decompressor::new(Cursor::new(body), 4096), max_output_bytes),
        other => Err(BrowserError::message(format!(
            "unsupported content encoding: {other}"
        ))),
    }
}

fn decode_deflate(body: Vec<u8>, max_output_bytes: Option<usize>) -> Result<Vec<u8>> {
    let first_try = read_all(
        ZlibDecoder::new(Cursor::new(body.clone())),
        max_output_bytes,
    );
    match first_try {
        Ok(decoded) => Ok(decoded),
        Err(_) => read_all(DeflateDecoder::new(Cursor::new(body)), max_output_bytes),
    }
}

fn read_all(reader: impl Read, max_output_bytes: Option<usize>) -> Result<Vec<u8>> {
    let mut reader = reader;
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        output.extend_from_slice(&chunk[..read]);
        if let Some(limit) = max_output_bytes
            && output.len() > limit
        {
            return Err(BrowserError::message(format!(
                "decoded response exceeded limit of {limit} bytes"
            )));
        }
    }
    Ok(output)
}

fn read_response_bytes(
    stream: &mut dyn ReadWrite,
    max_response_bytes: Option<usize>,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8192];

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                output.extend_from_slice(&chunk[..read]);
                if let Some(limit) = max_response_bytes
                    && output.len() > limit
                {
                    return Err(BrowserError::message(format!(
                        "raw response exceeded limit of {limit} bytes"
                    )));
                }
            }
            Err(error) if is_tls_close_without_notify(&error) => break,
            Err(error) => return Err(error.into()),
        }
    }

    Ok(output)
}

fn is_tls_close_without_notify(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::UnexpectedEof
        || error
            .to_string()
            .contains("peer closed connection without sending TLS close_notify")
}

fn decode_chunked(mut input: &[u8], max_output_bytes: Option<usize>) -> Result<Vec<u8>> {
    let mut output = Vec::new();

    loop {
        let Some(line_end) = find_bytes(input, b"\r\n") else {
            return Err(BrowserError::message(
                "invalid chunked response: missing chunk size line",
            ));
        };

        let size_line = std::str::from_utf8(&input[..line_end])
            .map_err(|_| BrowserError::message("invalid chunked response: size is not utf-8"))?;
        let size_text = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| BrowserError::message("invalid chunked response: bad chunk size"))?;

        input = &input[line_end + 2..];

        if size == 0 {
            break;
        }

        if input.len() < size + 2 {
            return Err(BrowserError::message(
                "invalid chunked response: truncated chunk body",
            ));
        }

        output.extend_from_slice(&input[..size]);
        if let Some(limit) = max_output_bytes
            && output.len() > limit
        {
            return Err(BrowserError::message(format!(
                "chunked response exceeded limit of {limit} bytes"
            )));
        }

        if &input[size..size + 2] != b"\r\n" {
            return Err(BrowserError::message(
                "invalid chunked response: missing chunk terminator",
            ));
        }

        input = &input[size + 2..];
    }

    Ok(output)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

trait ReadWrite: Read + Write {}

impl<T> ReadWrite for T where T: Read + Write {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::{
        HttpRequestOptions, build_request_bytes, decode_chunked, parse_response,
        parse_response_with_limits,
    };
    use crate::url::Url;

    #[test]
    fn decodes_chunked_bodies() {
        let bytes = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        let decoded = decode_chunked(bytes, None).unwrap();

        assert_eq!(decoded, b"Wikipedia");
    }

    #[test]
    fn parses_status_headers_and_body() {
        let url = Url::parse("http://example.com").unwrap();
        let response = parse_response(
            &url,
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello",
        )
        .unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(response.header("content-type"), Some("text/plain"));
        assert_eq!(response.body, b"hello");
    }

    #[test]
    fn parses_gzip_encoded_body() {
        let url = Url::parse("https://example.com").unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"hello gzip").unwrap();
        let body = encoder.finish().unwrap();

        let mut response_bytes =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\n\r\n"
                .to_vec();
        response_bytes.extend_from_slice(&body);

        let response = parse_response(&url, &response_bytes).unwrap();

        assert_eq!(response.body, b"hello gzip");
    }

    #[test]
    fn rejects_bodies_that_exceed_limit() {
        let url = Url::parse("https://example.com").unwrap();
        let response = parse_response_with_limits(
            &url,
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello world",
            Some(4),
        );

        assert!(response.is_err());
    }

    #[test]
    fn builds_post_request_with_headers_and_body() {
        let url = Url::parse("https://example.com/api").unwrap();
        let request = HttpRequestOptions {
            method: "post".to_string(),
            headers: BTreeMap::from([
                ("x-demo".to_string(), "one".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
            ]),
            body: Some(br#"{"ok":true}"#.to_vec()),
        };

        let request_bytes = build_request_bytes(&url, &request);
        let request_text = String::from_utf8(request_bytes).unwrap();

        assert!(
            request_text.starts_with("POST /api HTTP/1.1\r\n"),
            "{request_text}"
        );
        assert!(request_text.contains("x-demo: one") || request_text.contains("X-Demo: one"));
        assert!(
            request_text.contains("content-type: application/json"),
            "{request_text}"
        );
        assert!(
            request_text.contains("Content-Length: 11"),
            "{request_text}"
        );
        assert!(
            request_text.ends_with("\r\n\r\n{\"ok\":true}"),
            "{request_text}"
        );
    }
}
