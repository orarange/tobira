use crate::error::Result;
use crate::html::parse_document;
use crate::http::fetch;
use crate::render::render_document;
use crate::url::Url;

#[derive(Debug, Clone)]
pub struct BrowserPage {
    pub url: Url,
    pub status_code: u16,
    pub reason_phrase: String,
    pub content_type: Option<String>,
    pub rendered: String,
}

impl BrowserPage {
    pub fn status_text(&self) -> String {
        format!("{} {}", self.status_code, self.reason_phrase)
            .trim()
            .to_string()
    }

    pub fn body_text(&self) -> &str {
        let trimmed = self.rendered.trim();
        if trimmed.is_empty() {
            "[empty document]"
        } else {
            trimmed
        }
    }

    pub fn to_cli_output(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("URL: {}\n", self.url));
        output.push_str(&format!("Status: {}\n", self.status_text()));
        if let Some(content_type) = &self.content_type {
            output.push_str(&format!("Content-Type: {content_type}\n"));
        }
        output.push('\n');
        output.push_str(self.body_text());
        output.push('\n');
        output
    }
}

pub fn load_page(url: &Url) -> Result<BrowserPage> {
    let response = fetch(url)?;
    let content_type = response.header("content-type").map(str::to_string);
    let document = parse_document(&String::from_utf8_lossy(&response.body));
    let rendered = render_document(&document);

    Ok(BrowserPage {
        url: response.final_url,
        status_code: response.status_code,
        reason_phrase: response.reason_phrase,
        content_type,
        rendered,
    })
}

#[cfg(test)]
mod tests {
    use super::BrowserPage;
    use crate::url::Url;

    #[test]
    fn falls_back_when_document_is_empty() {
        let page = BrowserPage {
            url: Url::parse("http://example.com").unwrap(),
            status_code: 200,
            reason_phrase: "OK".to_string(),
            content_type: Some("text/html".to_string()),
            rendered: "   ".to_string(),
        };

        assert_eq!(page.body_text(), "[empty document]");
    }

    #[test]
    fn formats_cli_output() {
        let page = BrowserPage {
            url: Url::parse("http://example.com").unwrap(),
            status_code: 200,
            reason_phrase: "OK".to_string(),
            content_type: Some("text/html".to_string()),
            rendered: "# Hello".to_string(),
        };

        let output = page.to_cli_output();

        assert!(output.contains("URL: http://example.com/"));
        assert!(output.contains("Status: 200 OK"));
        assert!(output.contains("# Hello"));
    }
}
