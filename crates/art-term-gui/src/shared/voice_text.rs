//! Joins streaming SAPI transcript segments into single-line terminal text.

fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{3400}'..='\u{4dbf}' |
        '\u{4e00}'..='\u{9fff}' |
        '\u{f900}'..='\u{faff}' |
        '\u{20000}'..='\u{2a6df}' |
        '\u{2a700}'..='\u{2ebef}' |
        '\u{3000}'..='\u{303f}' |
        '\u{3040}'..='\u{30ff}' |
        '\u{31f0}'..='\u{31ff}' |
        '\u{ff00}'..='\u{ffef}' |
        '\u{ac00}'..='\u{d7af}'
    )
}

fn is_opening(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{' | '（' | '【' | '「' | '『' | '《' | '〈' | '“' | '‘' | '《')
}

fn is_closing_or_punctuation(ch: char) -> bool {
    matches!(ch,
        ')' | ']' | '}' | '，' | '。' | '、' | '；' | '：' | '！' | '？' |
        ',' | '.' | ';' | ':' | '!' | '?' |
        '）' | '】' | '」' | '』' | '》' | '〉' | '”' | '’' | '…' | '·' | '・'
    )
}

fn word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn needs_space(prev: &str, next: &str) -> bool {
    let Some(last) = prev.chars().next_back() else { return false };
    let Some(first) = next.chars().next() else { return false };
    if is_opening(last) || is_closing_or_punctuation(first) || is_closing_or_punctuation(last) {
        return false;
    }
    if is_cjk(last) || is_cjk(first) {
        return false;
    }
    word_char(last) && word_char(first)
}

/// Append one recognized utterance to the committed text, inserting a space
/// only where separating two Latin/alphanumeric words would otherwise merge.
pub fn append_segment(committed: &mut String, segment: &str) {
    let segment = segment.trim();
    if segment.is_empty() {
        return;
    }
    if committed.is_empty() {
        committed.push_str(segment);
        return;
    }
    if needs_space(committed, segment) {
        committed.push(' ');
    }
    committed.push_str(segment);
}

/// Text shown while recording: committed utterances plus the live hypothesis.
pub fn display_text(committed: &str, partial: &str) -> String {
    let partial = partial.trim();
    if partial.is_empty() {
        return committed.to_string();
    }
    let mut out = committed.to_string();
    append_segment(&mut out, partial);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_latin_words_with_spaces_and_cjk_without() {
        let mut text = String::new();
        append_segment(&mut text, "open");
        append_segment(&mut text, "the");
        append_segment(&mut text, "檔案");
        append_segment(&mut text, "and");
        append_segment(&mut text, "settings");
        assert_eq!(text, "open the檔案 and settings");
    }

    #[test]
    fn avoids_spaces_around_punctuation_and_brackets() {
        let mut text = String::new();
        append_segment(&mut text, "run");
        append_segment(&mut text, "(safe)");
        append_segment(&mut text, "now");
        append_segment(&mut text, "please");
        append_segment(&mut text, "，");
        append_segment(&mut text, "done");
        assert_eq!(text, "run (safe) now please，done");
    }

    #[test]
    fn display_text_shows_live_hypothesis_without_committing_it() {
        let mut committed = String::new();
        append_segment(&mut committed, "已送出");
        assert_eq!(display_text(&committed, " 下一句"), "已送出 下一句");
        assert_eq!(display_text(&committed, "  "), "已送出");
    }
}
