/// Make invisible formatting/control characters visible without changing API identifiers.
pub fn safe(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if matches!(c,'\u{061c}'|'\u{200b}'..='\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2060}'..='\u{206f}'|'\u{feff}')
            || (c.is_control() && !matches!(c, '\n' | '\t'))
        {
            out.push_str(&format!("<U+{:04X}>", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// Last-mile protection for all externally sourced labels and code spans.
pub fn sanitize_buffer(buffer: &mut ratatui::buffer::Buffer) {
    for cell in &mut buffer.content {
        let safe = safe(cell.symbol());
        if safe != cell.symbol() {
            cell.set_symbol("�");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invisible_controls_are_not_silently_rendered() {
        assert_eq!(safe("a\u{202e}b\u{200b}c"), "a<U+202E>b<U+200B>c");
        assert_eq!(safe("한글\nhello"), "한글\nhello");
    }
}
