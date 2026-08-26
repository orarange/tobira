use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use brotli::Decompressor;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use rustls_platform_verifier::ConfigVerifierExt;

use crate::error::{BrowserError, Result};
use crate::site_state;
use crate::url::Url;

const MAX_REDIRECTS: usize = 5;
use tobira_engine::engine::USER_AGENT;
const RESPONSE_HEADER_SLACK_BYTES: usize = 64 * 1024;
/// Cap on a single decoded response body. Applied by [`fetch`] so that no call
/// site can pull an unbounded response into memory, and so a small gzip/brotli
/// payload cannot expand without limit — `read_all` checks this while it
/// decompresses, not after, so a bomb is abandoned early. Callers that want a
/// tighter budget use [`fetch_with_limits`].
pub const DEFAULT_MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

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

pub fn fetch(url: &Url) -> Result<HttpResponse> {
    fetch_inner(url, 0, Some(DEFAULT_MAX_BODY_BYTES), None)
}

pub fn fetch_with_limits(url: &Url, max_body_bytes: usize) -> Result<HttpResponse> {
    fetch_inner(url, 0, Some(max_body_bytes), None)
}

pub fn fetch_with_limits_same_origin(
    url: &Url,
    max_body_bytes: usize,
    origin: &Url,
) -> Result<HttpResponse> {
    fetch_inner(url, 0, Some(max_body_bytes), Some(origin))
}

fn fetch_inner(
    url: &Url,
    redirect_count: usize,
    max_body_bytes: Option<usize>,
    same_origin: Option<&Url>,
) -> Result<HttpResponse> {
    if redirect_count > MAX_REDIRECTS {
        return Err(BrowserError::message("too many redirects"));
    }

    // A pooled connection may have been closed by the peer since we parked it,
    // and we only find that out when the write or read fails. That is expected,
    // not an error: fall back to a fresh connection once before giving up.
    let pooled_available = has_pooled(url);
    let response_bytes = match exchange(url, max_body_bytes, true) {
        Ok(bytes) => bytes,
        Err(_) if pooled_available => exchange(url, max_body_bytes, false)?,
        Err(error) => return Err(error),
    };

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
            return fetch_inner(&next_url, redirect_count + 1, max_body_bytes, same_origin);
        }
    }

    Ok(response)
}

/// Send one request and read one response, reusing a parked connection when
/// `allow_pooled` is set. The connection returns to the pool only when the
/// response was framed well enough to know we consumed exactly it and no more.
fn exchange(url: &Url, max_body_bytes: Option<usize>, allow_pooled: bool) -> Result<Vec<u8>> {
    let mut connection = match allow_pooled.then(|| take_pooled(url)).flatten() {
        Some(connection) => connection,
        None => {
            let address = format!("{}:{}", url.host, url.port);
            let tcp_stream = TcpStream::connect(address)?;
            tcp_stream.set_read_timeout(Some(Duration::from_secs(20)))?;
            tcp_stream.set_write_timeout(Some(Duration::from_secs(20)))?;
            Connection {
                stream: open_stream(url, tcp_stream)?,
                leftover: Vec::new(),
            }
        }
    };

    let cookie_header = site_state::cookie_header_for_url(url)
        .map(|value| format!("Cookie: {value}\r\n"))
        .unwrap_or_default();

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/png,*/*;q=0.8\r\nAccept-Language: ja-JP,ja;q=0.9,en-US;q=0.8,en;q=0.7\r\nAccept-Encoding: gzip, deflate, br\r\nCache-Control: no-cache\r\nPragma: no-cache\r\nConnection: keep-alive\r\nUpgrade-Insecure-Requests: 1\r\nSec-CH-UA: \"Chromium\";v=\"136\", \"Google Chrome\";v=\"136\", \"Not/A)Brand\";v=\"99\"\r\nSec-CH-UA-Mobile: ?0\r\nSec-CH-UA-Platform: \"Windows\"\r\nSec-Fetch-Dest: document\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Site: none\r\nSec-Fetch-User: ?1\r\n{cookie_header}\r\n",
        url.path,
        url.host_header(),
        USER_AGENT
    );

    connection.stream.write_all(request.as_bytes())?;
    connection.stream.flush()?;

    let carried = std::mem::take(&mut connection.leftover);
    let (bytes, leftover, reusable) = read_response_bytes(
        &mut *connection.stream,
        carried,
        max_body_bytes.map(|limit| limit.saturating_add(RESPONSE_HEADER_SLACK_BYTES)),
    )?;

    if reusable {
        connection.leftover = leftover;
        return_to_pool(url, connection);
    }
    Ok(bytes)
}

fn has_pooled(url: &Url) -> bool {
    connection_pool()
        .lock()
        .ok()
        .and_then(|pool| pool.get(&pool_key(url)).map(|entries| !entries.is_empty()))
        .unwrap_or(false)
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

/// Built once per process. `ClientConfig::with_platform_verifier()` enumerates
/// the OS certificate store, which costs seconds on Windows -- and this used to
/// run on *every* HTTPS request, so a page with a few dozen subresources spent
/// almost all of its load time rebuilding the same trust store.
static TLS_CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();

fn tls_config() -> Result<Arc<ClientConfig>> {
    if let Some(config) = TLS_CONFIG.get() {
        return Ok(Arc::clone(config));
    }
    let config = Arc::new(
        ClientConfig::with_platform_verifier()
            .map_err(|error| BrowserError::message(error.to_string()))?,
    );
    // A racing thread may have installed one first; either is equally valid.
    let _ = TLS_CONFIG.set(Arc::clone(&config));
    Ok(config)
}

fn open_stream(url: &Url, tcp_stream: TcpStream) -> Result<Box<dyn ReadWrite + Send>> {
    match url.scheme.as_str() {
        "http" => Ok(Box::new(tcp_stream)),
        "https" => {
            let config = tls_config()?;
            let server_name = ServerName::try_from(url.host.clone())
                .map_err(|_| BrowserError::message("invalid https host name"))?;
            let connection = ClientConnection::new(config, server_name)
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

/// How the peer told us the body ends.
enum BodyFraming {
    /// `Content-Length: n`
    Length(usize),
    /// `Transfer-Encoding: chunked`
    Chunked,
    /// Neither: the body runs until the connection closes, so the connection
    /// cannot be reused.
    UntilClose,
}

/// Minimal header scan over an already-complete header block: just the fields
/// that decide framing and reuse. Full parsing happens later in
/// `parse_response_with_limits`.
fn scan_framing(header_block: &[u8]) -> (BodyFraming, bool) {
    let text = String::from_utf8_lossy(header_block);
    let mut framing = BodyFraming::UntilClose;
    let mut wants_close = false;
    let mut status_is_bodyless = false;

    for (index, line) in text.split("\r\n").enumerate() {
        if index == 0 {
            // 204 and 304 never carry a body regardless of headers.
            if let Some(code) = line.split(' ').nth(1) {
                status_is_bodyless = matches!(code, "204" | "304");
            }
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name.as_str() {
            "transfer-encoding" if value.to_ascii_lowercase().contains("chunked") => {
                framing = BodyFraming::Chunked;
            }
            "content-length" => {
                if !matches!(framing, BodyFraming::Chunked)
                    && let Ok(length) = value.parse::<usize>()
                {
                    framing = BodyFraming::Length(length);
                }
            }
            "connection" if value.to_ascii_lowercase().contains("close") => {
                wants_close = true;
            }
            _ => {}
        }
    }

    if status_is_bodyless {
        framing = BodyFraming::Length(0);
    }
    let reusable = !wants_close && !matches!(framing, BodyFraming::UntilClose);
    (framing, reusable)
}

/// Total length of a complete chunked stream starting at `body`, or `None` if
/// more bytes are still needed.
fn chunked_stream_len(body: &[u8]) -> Option<usize> {
    let mut offset = 0usize;
    loop {
        let line_end = find_bytes(&body[offset..], b"\r\n")? + offset;
        let size_line = &body[offset..line_end];
        let size_text = String::from_utf8_lossy(size_line);
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).ok()?;
        offset = line_end + 2;
        if size == 0 {
            // Trailer section, ended by a blank line.
            let end = find_bytes(&body[offset..], b"\r\n")?;
            return Some(offset + end + 2);
        }
        offset = offset.checked_add(size)?;
        if body.len() < offset + 2 {
            return None;
        }
        offset += 2; // CRLF after the chunk data
    }
}

/// Read exactly one HTTP/1.1 response. Returns the raw bytes and whether the
/// connection is still in a known-good state afterwards, i.e. whether the body
/// was framed by `Content-Length` or chunked encoding and the peer did not ask
/// to close. Reading to EOF instead would consume the connection.
fn read_response_bytes(
    stream: &mut dyn ReadWrite,
    prefix: Vec<u8>,
    max_response_bytes: Option<usize>,
) -> Result<(Vec<u8>, Vec<u8>, bool)> {
    let mut output = prefix;
    let mut chunk = [0_u8; 8192];
    let mut eof = false;

    let over_limit = |len: usize| -> Result<()> {
        if let Some(limit) = max_response_bytes
            && len > limit
        {
            return Err(BrowserError::message(format!(
                "raw response exceeded limit of {limit} bytes"
            )));
        }
        Ok(())
    };

    let mut fill = |output: &mut Vec<u8>, eof: &mut bool| -> Result<()> {
        match stream.read(&mut chunk) {
            Ok(0) => *eof = true,
            Ok(read) => output.extend_from_slice(&chunk[..read]),
            Err(error) if is_tls_close_without_notify(&error) => *eof = true,
            Err(error) => return Err(error.into()),
        }
        Ok(())
    };

    // 1. headers
    let header_end = loop {
        if let Some(at) = find_bytes(&output, b"\r\n\r\n") {
            break at + 4;
        }
        if eof {
            return Err(BrowserError::message(
                "invalid HTTP response: missing header separator",
            ));
        }
        fill(&mut output, &mut eof)?;
        over_limit(output.len())?;
    };

    let (framing, reusable) = scan_framing(&output[..header_end.saturating_sub(4)]);

    // 2. body, bounded by whatever framing the peer chose
    match framing {
        BodyFraming::Length(length) => {
            let target = header_end.saturating_add(length);
            while output.len() < target && !eof {
                fill(&mut output, &mut eof)?;
                over_limit(output.len())?;
            }
            if output.len() < target {
                return Err(BrowserError::message(
                    "connection closed before the declared Content-Length was received",
                ));
            }
            let leftover = output.split_off(target);
            return Ok((output, leftover, reusable));
        }
        BodyFraming::Chunked => {
            while chunked_stream_len(&output[header_end..]).is_none() {
                if eof {
                    return Err(BrowserError::message(
                        "connection closed inside a chunked response",
                    ));
                }
                fill(&mut output, &mut eof)?;
                over_limit(output.len())?;
            }
            let body_len = chunked_stream_len(&output[header_end..]).unwrap_or(0);
            let leftover = output.split_off(header_end + body_len);
            return Ok((output, leftover, reusable));
        }
        BodyFraming::UntilClose => {
            while !eof {
                fill(&mut output, &mut eof)?;
                over_limit(output.len())?;
            }
        }
    }

    Ok((output, Vec::new(), reusable))
}

/// A live connection parked for reuse, with the moment it went idle so that
/// long-dead ones are dropped rather than handed out.
struct Connection {
    stream: Box<dyn ReadWrite + Send>,
    /// Bytes already pulled off the socket that belong to a later response.
    /// Reading is buffered, so finishing one response can over-read into the
    /// next; dropping those bytes would corrupt whatever comes after.
    leftover: Vec<u8>,
}

struct PooledConnection {
    connection: Connection,
    idle_since: Instant,
}

/// Servers close idle keep-alive connections on their own schedule; anything
/// older than this is assumed gone. A stale one that slips through is still
/// handled by the single retry in `fetch_inner`.
const MAX_IDLE: Duration = Duration::from_secs(5);
const MAX_POOLED_PER_HOST: usize = 6;

static CONNECTION_POOL: OnceLock<Mutex<HashMap<String, Vec<PooledConnection>>>> = OnceLock::new();

fn connection_pool() -> &'static Mutex<HashMap<String, Vec<PooledConnection>>> {
    CONNECTION_POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pool_key(url: &Url) -> String {
    format!("{}://{}:{}", url.scheme, url.host, url.port)
}

fn take_pooled(url: &Url) -> Option<Connection> {
    let mut pool = connection_pool().lock().ok()?;
    let entries = pool.get_mut(&pool_key(url))?;
    while let Some(entry) = entries.pop() {
        if entry.idle_since.elapsed() < MAX_IDLE {
            return Some(entry.connection);
        }
    }
    None
}

fn return_to_pool(url: &Url, connection: Connection) {
    let Ok(mut pool) = connection_pool().lock() else {
        return;
    };
    let entries = pool.entry(pool_key(url)).or_default();
    if entries.len() >= MAX_POOLED_PER_HOST {
        return;
    }
    entries.push(PooledConnection {
        connection,
        idle_since: Instant::now(),
    });
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
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::{
        BodyFraming, DEFAULT_MAX_BODY_BYTES, chunked_stream_len, decode_chunked, parse_response,
        parse_response_with_limits, read_response_bytes, scan_framing,
    };
    use crate::url::Url;

    /// A keep-alive connection is only safe to reuse if we consumed exactly one
    /// response, so framing detection is the load-bearing part of the pool.
    #[test]
    fn scans_content_length_framing() {
        let headers = b"HTTP/1.1 200 OK@@Content-Type: text/plain@@Content-Length: 42";
        let (framing, reusable) = scan_framing(&crlf(headers));
        assert!(matches!(framing, BodyFraming::Length(42)));
        assert!(reusable);
    }

    #[test]
    fn scans_chunked_framing_and_wins_over_content_length() {
        let headers = b"HTTP/1.1 200 OK@@Content-Length: 9@@Transfer-Encoding: chunked";
        let (framing, reusable) = scan_framing(&crlf(headers));
        assert!(matches!(framing, BodyFraming::Chunked));
        assert!(reusable);
    }

    #[test]
    fn connection_close_makes_a_response_non_reusable() {
        let headers = b"HTTP/1.1 200 OK@@Content-Length: 3@@Connection: close";
        let (_, reusable) = scan_framing(&crlf(headers));
        assert!(!reusable);
    }

    /// Without framing the body runs to EOF, so the socket is spent.
    #[test]
    fn missing_framing_is_read_until_close() {
        let headers = b"HTTP/1.1 200 OK@@Content-Type: text/plain";
        let (framing, reusable) = scan_framing(&crlf(headers));
        assert!(matches!(framing, BodyFraming::UntilClose));
        assert!(!reusable);
    }

    /// 204 and 304 carry no body even if a Content-Length says otherwise.
    #[test]
    fn bodyless_statuses_have_no_body() {
        for status in ["204 No Content", "304 Not Modified"] {
            let raw = format!("HTTP/1.1 {status}@@Content-Length: 100");
            let (framing, reusable) = scan_framing(&crlf(raw.as_bytes()));
            assert!(matches!(framing, BodyFraming::Length(0)), "{status}");
            assert!(reusable, "{status}");
        }
    }

    #[test]
    fn chunked_length_needs_the_terminating_chunk() {
        assert_eq!(chunked_stream_len(&crlf(b"4@@Wiki@@5@@pedia@@0@@@@")), Some(24));
        // Same stream with the trailer missing: not complete yet.
        assert_eq!(chunked_stream_len(&crlf(b"4@@Wiki@@5@@pedia@@0@@")), None);
        // Cut mid-chunk.
        assert_eq!(chunked_stream_len(&crlf(b"4@@Wi")), None);
    }

    /// The whole point of the framing work: read exactly one response and leave
    /// anything after it untouched, so the connection can serve the next one.
    #[test]
    fn reads_exactly_one_response_off_a_shared_stream() {
        let first = crlf(b"HTTP/1.1 200 OK@@Content-Length: 5@@@@hello");
        let second = crlf(b"HTTP/1.1 200 OK@@Content-Length: 5@@@@world");
        let mut both = first.clone();
        both.extend_from_slice(&second);

        let mut stream = MockStream::new(both);
        let (bytes, leftover, reusable) =
            read_response_bytes(&mut stream, Vec::new(), None).unwrap();
        assert_eq!(bytes, first, "must not swallow the following response");
        assert!(reusable);

        // Whatever was over-read has to come back so the next read can use it;
        // dropping it would corrupt the following response on this connection.
        let (bytes, _, _) = read_response_bytes(&mut stream, leftover, None).unwrap();
        assert_eq!(bytes, second, "the next response should still be there");
    }

    #[test]
    fn reads_exactly_one_chunked_response() {
        let body = crlf(b"HTTP/1.1 200 OK@@Transfer-Encoding: chunked@@@@4@@Wiki@@5@@pedia@@0@@@@");
        let mut trailing = body.clone();
        trailing.extend_from_slice(b"LEFTOVER");
        let mut stream = MockStream::new(trailing);

        let (bytes, leftover, reusable) =
            read_response_bytes(&mut stream, Vec::new(), None).unwrap();
        assert_eq!(bytes, body);
        assert!(reusable);

        // Over-read bytes come back rather than being dropped; anything not yet
        // pulled off the socket is still there. Together they must reconstruct
        // exactly what followed the response.
        let mut rest = leftover;
        std::io::Read::read_to_end(&mut stream, &mut rest).unwrap();
        assert_eq!(rest, b"LEFTOVER");
    }

    /// A truncated body must be an error rather than a silently short read that
    /// then gets parsed as if it were complete.
    #[test]
    fn a_short_body_is_an_error() {
        let raw = crlf(b"HTTP/1.1 200 OK@@Content-Length: 50@@@@only-a-little");
        let mut stream = MockStream::new(raw);
        assert!(read_response_bytes(&mut stream, Vec::new(), None).is_err());
    }

    /// Replace `@@` with CRLF so the fixtures above stay readable.
    fn crlf(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        let mut i = 0;
        while i < input.len() {
            if input[i] == b'@' && i + 1 < input.len() && input[i + 1] == b'@' {
                out.extend_from_slice(b"\r\n");
                i += 2;
            } else {
                out.push(input[i]);
                i += 1;
            }
        }
        out
    }

    /// An in-memory socket: hands out a few bytes at a time so the read loops
    /// have to reassemble, the way a real stream behaves.
    struct MockStream {
        data: Vec<u8>,
        position: usize,
    }

    impl MockStream {
        fn new(data: Vec<u8>) -> Self {
            Self { data, position: 0 }
        }
    }

    impl std::io::Read for MockStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let remaining = self.data.len() - self.position;
            if remaining == 0 {
                return Ok(0);
            }
            let take = remaining.min(buffer.len()).min(7);
            buffer[..take].copy_from_slice(&self.data[self.position..self.position + take]);
            self.position += take;
            Ok(take)
        }
    }

    impl std::io::Write for MockStream {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

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

    /// A compressed payload that expands far past the limit must be rejected
    /// while it decompresses. `fetch` passes `DEFAULT_MAX_BODY_BYTES`, so this
    /// bound is what stands between a decompression bomb and the heap.
    #[test]
    fn rejects_compressed_bodies_that_expand_past_the_limit() {
        let url = Url::parse("https://example.com").unwrap();
        // 4 MiB of zeroes gzips down to a few KiB.
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&vec![0_u8; 4 * 1024 * 1024]).unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(
            compressed.len() < 64 * 1024,
            "the bomb should be small on the wire, got {} bytes",
            compressed.len()
        );

        let mut response_bytes =
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\n\r\n"
                .to_vec();
        response_bytes.extend_from_slice(&compressed);

        // Under a small cap the expansion is abandoned...
        assert!(parse_response_with_limits(&url, &response_bytes, Some(64 * 1024)).is_err());
        // ...and under the default it decodes normally.
        let ok = parse_response_with_limits(&url, &response_bytes, Some(DEFAULT_MAX_BODY_BYTES))
            .expect("4 MiB is well under the default cap");
        assert_eq!(ok.body.len(), 4 * 1024 * 1024);
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
}
