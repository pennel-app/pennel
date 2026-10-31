use crate::error::PennelError;

/// Speech recognition the host OS offers on-device (Apple's dictation model on macOS), implemented
/// by the app because only the platform UI layer can reach it.
#[uniffi::export(with_foreign)]
pub trait SystemTranscriber: Send + Sync {
    /// `language` is an ISO code like "nl", or `None` for the user's preferred language.
    /// Vocabulary contains at most 100 normalized phrases of at most 128 characters.
    fn transcribe(
        &self,
        samples: Vec<f32>,
        sample_rate: u32,
        language: Option<String>,
        vocabulary: Vec<String>,
    ) -> Result<String, PennelError>;
}

pub(crate) fn bounded_vocabulary(terms: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    terms
        .iter()
        .filter(|term| term.len() <= 512)
        .map(|term| term.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|term| !term.is_empty() && term.chars().count() <= 128)
        .filter(|term| seen.insert(term.to_lowercase()))
        .take(100)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_vocabulary_preserves_names_and_bounds_untrusted_terms() {
        let mut input = vec![
            "  Pennel  Mobile  ".into(),
            "pennel mobile".into(),
            "\n".into(),
        ];
        input.push("x".repeat(129));
        input.extend((0..120).map(|index| format!("Naam {index}")));
        let vocabulary = bounded_vocabulary(&input);
        assert_eq!(vocabulary.len(), 100);
        assert_eq!(vocabulary[0], "Pennel Mobile");
        assert_eq!(vocabulary[99], "Naam 98");
        assert!(vocabulary.iter().all(|term| term.chars().count() <= 128));
    }
}
