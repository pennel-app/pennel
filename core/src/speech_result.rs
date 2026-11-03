#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum SpeechTimingKind {
    Untimed,
    Segment,
    Word,
    Token,
    Attributed,
}

/// Exact UTF-8 text boundaries and times relative to this recognition's input audio.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct SpeechTextSpan {
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
pub struct SpeechTranscript {
    pub text: String,
    pub spans: Vec<SpeechTextSpan>,
    pub timing_kind: SpeechTimingKind,
}

impl SpeechTranscript {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            spans: Vec::new(),
            timing_kind: SpeechTimingKind::Untimed,
        }
    }

    /// Unreliable timing loses its authority, never any of the recognized text.
    pub fn validated(
        text: String,
        spans: Vec<SpeechTextSpan>,
        timing_kind: SpeechTimingKind,
        audio_duration_ms: u64,
    ) -> Self {
        if spans.is_empty() || timing_kind == SpeechTimingKind::Untimed {
            return Self::plain(text);
        }
        let mut cursor = 0;
        let mut previous_start = 0;
        for span in &spans {
            let start = span.start_byte as usize;
            let end = span.end_byte as usize;
            if start != cursor
                || end <= start
                || end > text.len()
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
                || span.start_ms >= span.end_ms
                || span.end_ms > audio_duration_ms
                || span.start_ms < previous_start
            {
                return Self::plain(text);
            }
            cursor = end;
            previous_start = span.start_ms;
        }
        if cursor != text.len() {
            return Self::plain(text);
        }
        Self {
            text,
            spans,
            timing_kind,
        }
    }

    pub fn trimmed(self) -> Self {
        let left = self.text.len() - self.text.trim_start().len();
        let right = self.text.trim_end().len();
        if left == 0 && right == self.text.len() {
            return self;
        }
        if left >= right {
            return Self::plain("");
        }
        let text = self.text[left..right].to_owned();
        let spans = self
            .spans
            .into_iter()
            .filter_map(|mut span| {
                let start = (span.start_byte as usize).max(left);
                let end = (span.end_byte as usize).min(right);
                if start >= end {
                    return None;
                }
                span.start_byte = u32::try_from(start - left).ok()?;
                span.end_byte = u32::try_from(end - left).ok()?;
                Some(span)
            })
            .collect();
        Self::validated(text, spans, self.timing_kind, u64::MAX)
    }

    #[cfg(any(feature = "local-models", test))]
    pub(crate) fn from_parts(
        text: String,
        parts: &[(String, u64, u64)],
        kind: SpeechTimingKind,
        audio_duration_ms: u64,
    ) -> Self {
        if parts.is_empty() || text.len() > u32::MAX as usize {
            return Self::plain(text);
        }
        let mut cursor = 0;
        let mut spans = Vec::with_capacity(parts.len());
        for (part, start_ms, end_ms) in parts {
            let part = part.trim();
            if part.is_empty() {
                return Self::plain(text);
            }
            let Some(relative) = text[cursor..].find(part) else {
                return Self::plain(text);
            };
            let end = cursor + relative + part.len();
            if !text[cursor..cursor + relative]
                .chars()
                .all(char::is_whitespace)
            {
                return Self::plain(text);
            }
            spans.push(SpeechTextSpan {
                start_byte: cursor as u32,
                end_byte: end as u32,
                start_ms: *start_ms,
                end_ms: *end_ms,
            });
            cursor = end;
        }
        if !text[cursor..].chars().all(char::is_whitespace) {
            return Self::plain(text);
        }
        if let Some(last) = spans.last_mut() {
            last.end_byte = text.len() as u32;
        }
        Self::validated(text, spans, kind, audio_duration_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_unicode_text_and_real_boundaries_are_conserved() {
        let text = "café 👩🏽‍💻, ja.";
        let split = "café ".len() as u32;
        let transcript = SpeechTranscript::validated(
            text.into(),
            vec![
                SpeechTextSpan {
                    start_byte: 0,
                    end_byte: split,
                    start_ms: 0,
                    end_ms: 500,
                },
                SpeechTextSpan {
                    start_byte: split,
                    end_byte: text.len() as u32,
                    start_ms: 450,
                    end_ms: 1200,
                },
            ],
            SpeechTimingKind::Word,
            1200,
        );
        assert_eq!(transcript.text, text);
        assert_eq!(transcript.spans.len(), 2);
        assert_eq!(transcript.spans[1].start_ms, 450);
        let joined: String = transcript
            .spans
            .iter()
            .map(|span| &text[span.start_byte as usize..span.end_byte as usize])
            .collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn malformed_or_partial_byte_and_time_ranges_keep_the_full_text_untimed() {
        let text = "café ja";
        for spans in [
            vec![SpeechTextSpan {
                start_byte: 0,
                end_byte: text.len() as u32,
                start_ms: 0,
                end_ms: 0,
            }],
            vec![SpeechTextSpan {
                start_byte: 0,
                end_byte: 4,
                start_ms: 0,
                end_ms: 900,
            }],
            vec![SpeechTextSpan {
                start_byte: 0,
                end_byte: text.len() as u32,
                start_ms: 901,
                end_ms: 900,
            }],
            vec![SpeechTextSpan {
                start_byte: 0,
                end_byte: text.len() as u32,
                start_ms: 0,
                end_ms: 1001,
            }],
        ] {
            let transcript =
                SpeechTranscript::validated(text.into(), spans, SpeechTimingKind::Word, 1000);
            assert_eq!(transcript, SpeechTranscript::plain(text));
        }
    }

    #[test]
    fn sequential_parts_only_map_exact_text_without_inventing_punctuation() {
        let text = "  Hallo, café.  ";
        let valid = SpeechTranscript::from_parts(
            text.into(),
            &[("Hallo,".into(), 0, 400), ("café.".into(), 450, 900)],
            SpeechTimingKind::Word,
            1000,
        );
        assert_eq!(valid.spans.len(), 2);
        assert_eq!(valid.trimmed().text, "Hallo, café.");
        let invalid = SpeechTranscript::from_parts(
            text.into(),
            &[("Hallo".into(), 0, 400), ("café".into(), 450, 900)],
            SpeechTimingKind::Word,
            1000,
        );
        assert_eq!(invalid, SpeechTranscript::plain(text));
    }

    #[test]
    fn trimming_adjusts_bytes_without_adding_the_meeting_clock() {
        let text = "  café 👩🏽‍💻  ";
        let transcript = SpeechTranscript::validated(
            text.into(),
            vec![SpeechTextSpan {
                start_byte: 0,
                end_byte: text.len() as u32,
                start_ms: 125,
                end_ms: 875,
            }],
            SpeechTimingKind::Segment,
            1000,
        )
        .trimmed();
        assert_eq!(transcript.text, "café 👩🏽‍💻");
        assert_eq!(transcript.spans[0].start_byte, 0);
        assert_eq!(transcript.spans[0].end_byte as usize, transcript.text.len());
        assert_eq!(
            (transcript.spans[0].start_ms, transcript.spans[0].end_ms),
            (125, 875)
        );
    }
}
