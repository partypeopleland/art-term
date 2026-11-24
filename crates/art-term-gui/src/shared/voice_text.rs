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
    matches!(
        ch,
        '(' | '[' | '{' | '（' | '【' | '「' | '『' | '《' | '〈' | '“' | '‘'
    )
}

fn is_closing(ch: char) -> bool {
    matches!(
        ch,
        ')' | ']' | '}' | '）' | '】' | '」' | '』' | '》' | '〉' | '”' | '’'
    )
}

fn is_clause_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '！'
            | '？'
            | ','
            | '.'
            | ';'
            | ':'
            | '!'
            | '?'
            | '…'
            | '·'
            | '・'
    )
}

fn word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn needs_space(prev: &str, next: &str) -> bool {
    let Some(last) = prev.chars().next_back() else {
        return false;
    };
    let Some(first) = next.chars().next() else {
        return false;
    };
    // Brackets and punctuation hug the text they attach to: no space after an
    // opening mark or before a closing mark / clause punctuation.
    if is_opening(last) || is_closing(first) || is_clause_punctuation(first) {
        return false;
    }
    // CJK text never receives an inserted half-width space, including after
    // fullwidth punctuation ("已送出，下一句").
    if is_cjk(last) && is_cjk(first) {
        return false;
    }
    // After punctuation, separate the next word or opening bracket from the
    // mark itself ("hello, world", "， next").
    if is_clause_punctuation(last) {
        return word_char(first) || is_opening(first);
    }
    // Latin-alphanumeric adjacency and Latin/CJK script boundaries both need a
    // space so words cannot merge ("open the", "the 檔案", "檔案 and").
    if word_char(last) && word_char(first) {
        return true;
    }
    // Keep the space before an opening bracket ("run (safe)") and after a
    // closing bracket ("(safe) now") when they meet word text.
    if word_char(last) && is_opening(first) {
        return true;
    }
    if is_closing(last) && word_char(first) {
        return true;
    }
    false
}

/// Append one recognized utterance to the committed text. A half-width space
/// is inserted only where omitting it would merge words or detach brackets;
/// CJK characters stay unspaced and pauses do not become punctuation.
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
    fn joins_latin_words_and_separates_cjk_script_boundaries() {
        let mut text = String::new();
        append_segment(&mut text, "open");
        append_segment(&mut text, "the");
        append_segment(&mut text, "檔案");
        append_segment(&mut text, "and");
        append_segment(&mut text, "settings");
        assert_eq!(text, "open the 檔案 and settings");
    }

    #[test]
    fn joins_cjk_segments_without_inserted_spaces() {
        let mut text = String::new();
        append_segment(&mut text, "請幫我");
        append_segment(&mut text, "打開");
        append_segment(&mut text, "檔案");
        append_segment(&mut text, "，");
        append_segment(&mut text, "然後");
        append_segment(&mut text, "執行");
        append_segment(&mut text, "tests");
        assert_eq!(text, "請幫我打開檔案，然後執行 tests");
    }

    #[test]
    fn separates_latin_cjk_boundaries_in_both_directions() {
        let mut text = String::new();
        append_segment(&mut text, "使用");
        append_segment(&mut text, "VoiceOver");
        append_segment(&mut text, "設定");
        append_segment(&mut text, "enable");
        append_segment(&mut text, "中文");
        assert_eq!(text, "使用 VoiceOver 設定 enable 中文");
    }

    #[test]
    fn avoids_spaces_inside_split_brackets() {
        let mut text = String::new();
        append_segment(&mut text, "run");
        append_segment(&mut text, "(");
        append_segment(&mut text, "safe");
        append_segment(&mut text, ")");
        append_segment(&mut text, "now");
        assert_eq!(text, "run (safe) now");

        let mut text = String::new();
        append_segment(&mut text, "請看");
        append_segment(&mut text, "「");
        append_segment(&mut text, "測試");
        append_segment(&mut text, "」");
        append_segment(&mut text, "結果");
        assert_eq!(text, "請看「測試」結果");
    }

    #[test]
    fn separates_latin_words_around_punctuation() {
        let mut text = String::new();
        append_segment(&mut text, "hello");
        append_segment(&mut text, ",");
        append_segment(&mut text, "world");
        append_segment(&mut text, "please");
        append_segment(&mut text, "，");
        append_segment(&mut text, "done");
        assert_eq!(text, "hello, world please， done");
    }

    #[test]
    fn display_text_shows_live_hypothesis_without_committing_it() {
        let mut committed = String::new();
        append_segment(&mut committed, "已送出");
        assert_eq!(display_text(&committed, " 下一句"), "已送出下一句");
        assert_eq!(display_text(&committed, "  "), "已送出");
    }
}
