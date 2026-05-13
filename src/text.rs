use encoding_rs::Encoding;

pub fn decode_text_response(body: &[u8], content_type: Option<&str>) -> String {
    let charset = content_type
        .and_then(charset_from_content_type)
        .or_else(|| sniff_charset(body));

    let Some(charset) = charset else {
        return String::from_utf8_lossy(body).into_owned();
    };

    let Some(encoding) = Encoding::for_label(charset.as_bytes()) else {
        return String::from_utf8_lossy(body).into_owned();
    };

    let (decoded, _, _) = encoding.decode(body);
    decoded.into_owned()
}

pub fn charset_from_content_type(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|segment| {
        let (name, value) = segment.trim().split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("charset") {
            return None;
        }

        let value = value.trim().trim_matches('"').trim_matches('\'');
        (!value.is_empty()).then(|| value.to_string())
    })
}

pub fn sniff_charset(body: &[u8]) -> Option<String> {
    let sample = body
        .iter()
        .take(4096)
        .map(|byte| if byte.is_ascii() { *byte as char } else { ' ' })
        .collect::<String>()
        .to_ascii_lowercase();

    if let Some(index) = sample.find("charset=") {
        let rest = &sample[index + "charset=".len()..];
        let charset = rest
            .trim_start_matches(['"', '\'', ' '])
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
            .collect::<String>();
        if !charset.is_empty() {
            return Some(charset);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{charset_from_content_type, decode_text_response};

    #[test]
    fn decodes_shift_jis_using_meta_sniff() {
        let (encoded, _, _) = encoding_rs::SHIFT_JIS.encode(
            "<meta http-equiv=\"Content-Type\" content=\"text/html; charset=Shift_JIS\">阿部寛",
        );

        let decoded = decode_text_response(&encoded, None);

        assert!(decoded.contains("阿部寛"));
    }

    #[test]
    fn extracts_charset_from_content_type() {
        assert_eq!(
            charset_from_content_type("text/html; charset=Shift_JIS"),
            Some("Shift_JIS".to_string())
        );
        assert_eq!(charset_from_content_type("text/plain"), None);
    }
}
