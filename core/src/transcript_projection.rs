use crate::{MeetingUtterance, PennelError, utterance_evidence_revision};

pub(crate) fn rows(utterances: &[MeetingUtterance]) -> Result<Vec<serde_json::Value>, PennelError> {
    utterances
        .iter()
        .map(|row| {
            let mut value = serde_json::to_value(&row.segment).map_err(PennelError::storage)?;
            let object = value.as_object_mut().expect("segment object");
            object.insert(
                "utterance_id".into(),
                row.metadata.utterance_id.clone().into(),
            );
            object.insert(
                "source_id".into(),
                serde_json::to_value(&row.metadata.source_id).map_err(PennelError::storage)?,
            );
            object.insert(
                "source_fingerprint".into(),
                utterance_evidence_revision(row).into(),
            );
            let manual = row.metadata.manual.as_ref().filter(|manual| {
                !manual.review_required
                    && manual.source_fingerprint == row.metadata.source_fingerprint
            });
            let needs_review = row.metadata.manual.as_ref().is_some_and(|stored| {
                stored.review_required
                    || stored.source_fingerprint != row.metadata.source_fingerprint
            }) || !row.metadata.unresolved_manual.is_empty();
            let attribution = if let Some(voice) = manual.and_then(|manual| manual.voice) {
                serde_json::json!({"kind":"single","voices":[voice],"self_voice":false,"provisional":false,"turns":[]})
            } else if manual.is_none() && needs_review {
                serde_json::json!({"kind":"unknown","voices":[],"self_voice":false,"provisional":false,"turns":[]})
            } else {
                serde_json::to_value(&row.metadata.attribution).map_err(PennelError::storage)?
            };
            object.insert("attribution".into(), attribution);
            object.insert(
                "confirmed_speaker_name".into(),
                serde_json::to_value(manual.and_then(|manual| manual.name.as_deref()))
                    .map_err(PennelError::storage)?,
            );
            object.insert(
                "attribution_revision".into(),
                row.metadata.attribution_revision.into(),
            );
            object.insert("correction_review_required".into(), needs_review.into());
            Ok(value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttributionKind, AutomaticAttribution, ManualCorrection, Segment, Speaker};

    #[test]
    fn an_unflagged_stale_manual_binding_is_visible_for_review() {
        let mut row = MeetingUtterance::new(
            Segment {
                start_ms: 0,
                end_ms: 500,
                speaker: Speaker::Others,
                text: "Owned source.".into(),
                failed: false,
                voice: Some(2),
            },
            Some("microphone".into()),
        );
        row.metadata.attribution = AutomaticAttribution {
            kind: AttributionKind::Single,
            voices: vec![2],
            self_voice: false,
            provisional: false,
            turns: Vec::new(),
        };
        row.metadata.manual = Some(ManualCorrection {
            name: Some("Confirmed person".into()),
            voice: Some(1),
            source_fingerprint: "different source".into(),
            review_required: false,
        });
        let projected = rows(&[row]).unwrap();
        assert_eq!(projected[0]["correction_review_required"], true);
        assert_eq!(projected[0]["attribution"]["kind"], "unknown");
        assert_eq!(
            projected[0]["confirmed_speaker_name"],
            serde_json::Value::Null
        );
    }
}
