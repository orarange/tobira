use std::collections::HashMap;
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
use crate::url::Url;

const MAX_REDIRECTS: usize = 5;
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) ScratchBrowser/0.1 Safari/537.36";

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
    let tcp_stream = TcpStream::connect(address)?;
    tcp_stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    tcp_stream.set_write_timeout(Some(Duration::from_secs(20)))?;
    let mut stream = open_stream(url, tcp_stream)?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: text/html,application/xhtml+xml,application/xml;q=0.9,text/plain;q=0.8,*/*;q=0.5\r\nAccept-Language: en-US,en;q=0.9,ja;q=0.8\r\nAccept-Encoding: gzip, deflate, br\r\nConnection: close\r\nUpgrade-Insecure-Requests: 1\r\n\r\n",
        url.path,
        url.host_header(),
        USER_AGENT
    );

    stream.write_all(request.as_bytes())?;

    let response_bytes = read_response_bytes(&mut stream)?;

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
    let body = decode_content(body, headers.get("content-encoding"))?;

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

fn decode_content(body: Vec<u8>, content_encoding: Option<&String>) -> Result<Vec<u8>> {
    let Some(encoding) = content_encoding else {
        return Ok(body);
    };

    let encoding = encoding.to_ascii_lowercase();
    let primary = encoding.split(',').next().unwrap_or("").trim();

    match primary {
        "" | "identity" => Ok(body),
        "gzip" => read_all(GzDecoder::new(Cursor::new(body))),
        "deflate" => decode_deflate(body),
        "br" => read_all(Decompressor::new(Cursor::new(body), 4096)),
        other => Err(BrowserError::message(format!(
            "unsupported content encoding: {other}"
        ))),
    }
}

fn decode_deflate(body: Vec<u8>) -> Result<Vec<u8>> {
    let first_try = read_all(ZlibDecoder::new(Cursor::new(body.clone())));
    match first_try {
        Ok(decoded) => Ok(decoded),
        Err(_) => read_all(DeflateDecoder::new(Cursor::new(body))),
    }
}

fn read_all(reader: impl Read) -> Result<Vec<u8>> {
    let mut reader = reader;
    let mut output = Vec::new();
    reader.read_to_end(&mut output)?;
    Ok(output)
}

fn read_response_bytes(stream: &mut dyn ReadWrite) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8192];

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => output.extend_from_slice(&chunk[..read]),
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

trait ReadWrite: Read + Write {}

impl<T> ReadWrite for T where T: Read + Write {}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

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
}
