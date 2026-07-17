use console::strip_ansi_codes;

pub fn ansi_len(ansi_str: &str) -> usize {
    strip_ansi_codes(ansi_str).chars().count()
}

pub fn lpad_ansi(ansi_str: &str, len: usize) -> String {
    let stripped_len = ansi_len(ansi_str);
    let mut padded = ansi_str.to_string();
    padded.push_str(&" ".repeat(len.saturating_sub(stripped_len)));
    padded
}

#[cfg(test)]
mod tests {
    use super::*;
    use console::style;

    #[test]
    fn ansi_len_ignores_ansi_escape_codes() {
        let text = style("hello").red().bold().to_string();

        assert_eq!(ansi_len(&text), 5);
    }

    #[test]
    fn ansi_len_counts_unicode_chars() {
        assert_eq!(ansi_len("⠋ok"), 3);
    }

    #[test]
    fn lpad_ansi_pads_to_visible_width() {
        let text = style("up").green().to_string();

        assert_eq!(lpad_ansi(&text, 5), format!("{}   ", text));
    }

    #[test]
    fn lpad_ansi_does_not_truncate_when_text_is_wider() {
        assert_eq!(lpad_ansi("service", 3), "service");
    }
}
