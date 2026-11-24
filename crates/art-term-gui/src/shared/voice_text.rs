//! Joins streaming SAPI transcript segments with the same rule Android uses
//! for pause-separated dictation segments (`joinSegmentTexts` in
//! `VoiceSession.kt`): segments are appended verbatim and a fullwidth comma
//! is inserted between them unless the accumulated text already ends with
//! sentence punctuation. This keeps the pause behaviour identical across
//! platforms instead of inventing a different spacing rule for Windows.

/// Mirrors Android `SEGMENT_END_PUNCT`: punctuation that already closes a
/// segment, so no extra comma is added before the next one.
const SEGMENT_END_PUNCT: &[char] = &[
    '，', '。', '！', '？', '、', '；', '：', '…', ',', '.', '!', '?', ';', ':',
];

fn ends_with_segment_punctuation(text: &str) -> bool {
    text.chars()
        .rev()
        .find(|ch| !ch.is_whitespace())
        .is_some_and(|ch| SEGMENT_END_PUNCT.contains(&ch))
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
    if !ends_with_segment_punctuation(committed) {
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
    fn segments_are_appended_verbatim_without_inner_spaces() {
        // The join rule never inserts spaces, so brackets and punctuation
        // inside a segment — or split across segments — keep their text.
        let mut text = String::new();
        append_segment(&mut text, "run (safe) now");
        append_segment(&mut text, "「測試」");
        assert_eq!(text, "run (safe) now，「測試」");
    }

    #[test]
    fn brackets_split_across_segments_never_gain_inner_spaces() {
        // Even when "(", "safe" and ")" arrive as separate pause segments,
        // the join inserts only the Android-style comma — never whitespace.
        let mut text = String::new();
        for segment in ["run", "(", "safe", ")", "now"] {
            append_segment(&mut text, segment);
        }
        assert_eq!(text, "run，(，safe，)，now");
        assert!(!text.contains(" ("));
        assert!(!text.contains("( "));
        assert!(!text.contains(" )"));
        assert!(!text.contains(") "));
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
