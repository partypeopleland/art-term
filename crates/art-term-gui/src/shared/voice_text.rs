//! Joins streaming SAPI transcript segments with the same base rule Android
//! uses for pause-separated dictation segments (`joinSegmentTexts` in
//! `VoiceSession.kt`): segments are appended verbatim and a fullwidth comma
//! is inserted between them unless punctuation makes it redundant. On top of
//! Android's "already ends with sentence punctuation" check, brackets and
//! quotes suppress the comma on both sides so "( safe )" or 「」 never gain
//! stray marks when a pause splits them across segments.

/// Mirrors Android `SEGMENT_END_PUNCT`: punctuation that already closes a
/// segment, so no extra comma is added before the next one. Also used for a
/// segment that *starts* with punctuation — it attaches to the previous text.
const SEGMENT_END_PUNCT: &[char] = &[
    '，', '。', '！', '？', '、', '；', '：', '…', ',', '.', '!', '?', ';', ':',
];

fn is_clause_punctuation(ch: char) -> bool {
    SEGMENT_END_PUNCT.contains(&ch)
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

fn last_significant(text: &str) -> Option<char> {
    text.chars().rev().find(|ch| !ch.is_whitespace())
}

/// Whether a fullwidth comma should separate `committed` and `segment`.
/// Mirrors Android's rule (skip when the accumulated text already ends with
/// sentence punctuation) and extends it to bracket/quote boundaries: no comma
/// after an opening mark or before a closing mark, and never before
/// punctuation that belongs to the previous clause.
fn needs_comma(committed: &str, segment: &str) -> bool {
    let (Some(last), Some(first)) = (last_significant(committed), segment.chars().next()) else {
        return false;
    };
    !is_clause_punctuation(last)
        && !is_opening(last)
        && !is_closing(first)
        && !is_clause_punctuation(first)
}

/// Append one finalized utterance to the committed text.
pub fn append_segment(committed: &mut String, segment: &str) {
    let segment = segment.trim();
    if segment.is_empty() {
        return;
    }
    if committed.is_empty() {
        committed.push_str(segment);
        return;
    }
    if needs_comma(committed, segment) {
        committed.push('，');
    }
    committed.push_str(segment);
}

/// Text shown while recording: committed utterances plus the live hypothesis,
/// joined by the same rule the final text will use.
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
    fn joins_pause_segments_with_fullwidth_commas_like_android() {
        let mut text = String::new();
        for segment in ["請幫我", "開啟", "src/main.ts", "並且", "執行測試"] {
            append_segment(&mut text, segment);
        }
        assert_eq!(text, "請幫我，開啟，src/main.ts，並且，執行測試");
    }

    #[test]
    fn does_not_duplicate_existing_terminal_punctuation() {
        let mut text = String::new();
        for segment in ["你好。", "世界", "請說明，", "繼續"] {
            append_segment(&mut text, segment);
        }
        assert_eq!(text, "你好。世界，請說明，繼續");
    }

    #[test]
    fn applies_the_same_rule_to_latin_segments_as_android() {
        let mut text = String::new();
        for segment in ["open", "the", "file"] {
            append_segment(&mut text, segment);
        }
        assert_eq!(text, "open，the，file");
    }

    #[test]
    fn brackets_split_across_segments_stay_inside_the_commas() {
        // "(" suppresses the comma after it, ")" suppresses the comma before
        // it, so the parenthetical reads as one clause.
        let mut text = String::new();
        for segment in ["run", "(", "safe", ")", "now"] {
            append_segment(&mut text, segment);
        }
        assert_eq!(text, "run，(safe)，now");
    }

    #[test]
    fn quotes_split_across_segments_stay_inside_the_commas() {
        let mut text = String::new();
        for segment in ["他說", "「", "測試", "」", "結束"] {
            append_segment(&mut text, segment);
        }
        assert_eq!(text, "他說，「測試」，結束");
    }

    #[test]
    fn a_segment_starting_with_punctuation_attaches_without_comma() {
        let mut text = String::new();
        for segment in ["please", "，", "done"] {
            append_segment(&mut text, segment);
        }
        assert_eq!(text, "please，done");
    }

    #[test]
    fn segments_are_appended_verbatim_without_inner_spaces() {
        // The join rule never inserts spaces, so brackets and punctuation
        // inside a segment keep their text.
        let mut text = String::new();
        append_segment(&mut text, "run (safe) now");
        append_segment(&mut text, "「測試」");
        assert_eq!(text, "run (safe) now，「測試」");
    }

    #[test]
    fn display_text_previews_the_same_join_for_the_live_hypothesis() {
        let mut committed = String::new();
        append_segment(&mut committed, "已送出");
        assert_eq!(display_text(&committed, "下一句"), "已送出，下一句");
        assert_eq!(display_text(&committed, "  "), "已送出");
        assert_eq!(display_text("", "第一段"), "第一段");
    }
}
