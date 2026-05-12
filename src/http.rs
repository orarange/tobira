use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::error::{BrowserError, Result};
use crate::url::Url;

const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub final_url: Url,
    pub status_code: u16,
    pub reason_phrase: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        let key = name.to_ascii_lowercase();
        self.headers.get(&key).map(String::as_str)
    }
}

pub fn fetch(url: &Url) -> Result<HttpResponse> {
    fetch_inner(url, 0)
}

fn fetch_inner(url: &Url, redirect_count: usize) -> Result<HttpResponse> {
    if redirect_count > MAX_REDIRECTS {
        return Err(BrowserError::message("too many redirects"));
    }

    let address = format!("{}:{}", url.host, url.port);
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: scratch-browser/0.1\r\nAccept: text/html, text/plain;q=0.9, */*;q=0.1\r\nConnection: close\r\n\r\n",
        url.path,
        url.host_header()
    );

    stream.write_all(request.as_bytes())?;

    let mut response_bytes = Vec::new();
    stream.read_to_end(&mut response_bytes)?;

    let response = parse_response(url, &response_bytes)?;

    if is_redirect(response.status_code) {
        if let Some(location) = response.header("location") {
            let next_url = url.resolve(location)?;
            return fetch_inner(&next_url, redirect_count + 1);
        }
    }

    Ok(response)
}

fn parse_response(url: &Url, bytes: &[u8]) -> Result<HttpResponse> {
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
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let body = match headers.get("transfer-encoding") {
        Some(value) if value.to_ascii_lowercase().contains("chunked") => {
            decode_chunked(body_bytes)?
        }
        _ => body_bytes.to_vec(),
    };

    Ok(HttpResponse {
        final_url: url.clone(),
        status_code,
        reason_phrase,
        headers,
        body,
    })
}

fn is_redirect(status_code: u16) -> bool {
    matches!(status_code, 301 | 302 | 303 | 307 | 308)
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>> {
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

#[cfg(test)]
mod tests {
    use super::{decode_chunked, parse_response};
    use crate::url::Url;

    #[test]
    fn decodes_chunked_bodies() {
        let bytes = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        let decoded = decode_chunked(bytes).unwrap();

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
}
