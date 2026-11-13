use std::collections::HashSet;

use crate::{DecisionRequest, PennelError};

use super::invalid;

const FORMAT: &str = "pennel-local-independent-window-batch-v1";
const COMPONENTS: [&str; 8] = [
    "action",
    "decision",
    "open_question",
    "changed_commitment",
    "action_support",
    "decision_support",
    "open_question_support",
    "changed_commitment_support",
];

pub(super) struct ScopedWindow {
    pub request: DecisionRequest,
    pub original_indices: Vec<usize>,
}

fn exact_components(request: &DecisionRequest) -> bool {
    request.questions.len() == 8
        && request
            .questions
            .iter()
            .map(|question| question.id.as_str())
            .collect::<HashSet<_>>()
            == COMPONENTS.into_iter().collect()
}

/// Only the authored local envelope grants window grouping. Source text is never
/// interpreted as instructions or stripped by heuristic substring matching.
pub(super) fn windows(request: &DecisionRequest) -> Result<Vec<ScopedWindow>, PennelError> {
    super::validate_request(request)?;
    if exact_components(request) {
        return Ok(vec![ScopedWindow {
            request: request.clone(),
            original_indices: (0..8).collect(),
        }]);
    }
    let envelope: serde_json::Value = serde_json::from_str(&request.state)
        .map_err(|_| invalid("The local Decider requires independently scoped source windows."))?;
    let object = envelope
        .as_object()
        .filter(|object| object.len() == 2)
        .ok_or_else(|| invalid("The local Decider window envelope is invalid."))?;
    if object.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT) {
        return Err(invalid("The local Decider window profile is unsupported."));
    }
    let states = object
        .get("windows")
        .and_then(serde_json::Value::as_object)
        .filter(|states| (2..=8).contains(&states.len()))
        .ok_or_else(|| invalid("The local Decider window count is invalid."))?;
    if request.questions.len() != states.len() * 8 {
        return Err(invalid(
            "The local Decider window question set is incomplete.",
        ));
    }
    let mut windows = Vec::new();
    let mut assigned = HashSet::new();
    for slot in 0..states.len() {
        let key = format!("w{slot}");
        let prefix = format!("{key}_");
        let state = states
            .get(&key)
            .and_then(serde_json::Value::as_str)
            .filter(|state| !state.is_empty())
            .ok_or_else(|| invalid("The local Decider window source is invalid."))?;
        let mut questions = Vec::new();
        let mut indices = Vec::new();
        for (index, question) in request.questions.iter().enumerate() {
            if let Some(id) = question.id.strip_prefix(&prefix) {
                if !COMPONENTS.contains(&id)
                    || question.instructions.starts_with("Use only WINDOW ")
                    || !assigned.insert(index)
                {
                    return Err(invalid("The local Decider window namespace is invalid."));
                }
                let mut question = question.clone();
                question.id = id.into();
                questions.push(question);
                indices.push(index);
            }
        }
        let scoped = DecisionRequest {
            state: state.into(),
            questions,
        };
        if !exact_components(&scoped) {
            return Err(invalid(
                "The local Decider window components are incomplete.",
            ));
        }
        windows.push(ScopedWindow {
            request: scoped,
            original_indices: indices,
        });
    }
    if assigned.len() != request.questions.len() {
        return Err(invalid("The local Decider window namespace is incomplete."));
    }
    Ok(windows)
}

/// Flash v1.2's published contract recommends one question per evaluation.
/// Keep the logical component/backmap while isolating every native prompt.
pub(super) fn questions(request: &DecisionRequest) -> Result<Vec<ScopedWindow>, PennelError> {
    Ok(windows(request)?
        .into_iter()
        .flat_map(|window| {
            window
                .request
                .questions
                .into_iter()
                .zip(window.original_indices)
                .map(move |(question, index)| ScopedWindow {
                    request: DecisionRequest {
                        state: window.request.state.clone(),
                        questions: vec![question],
                    },
                    original_indices: vec![index],
                })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn question(id: &str) -> crate::DecisionQuestion {
        crate::DecisionQuestion {
            id: id.into(),
            kind: crate::DecisionQuestionKind::Choice,
            instructions: "Only SOURCE 0; neighbouring text supplies context.".into(),
            options: vec![
                crate::DecisionOption {
                    id: "yes".into(),
                    description: "Explicit".into(),
                },
                crate::DecisionOption {
                    id: "no".into(),
                    description: "Absent".into(),
                },
            ],
        }
    }
    fn request() -> DecisionRequest {
        DecisionRequest{state:serde_json::json!({"format":FORMAT,"windows":{"w0":"SOURCE 0: Synthetic action.","w1":"SOURCE 0: Synthetic decision."}}).to_string(),
            questions:(0..2).flat_map(|slot|COMPONENTS.map(move|id|question(&format!("w{slot}_{id}")))).collect()}
    }
    #[test]
    fn independent_windows_preserve_only_their_own_state_and_exact_backmap() {
        let request = request();
        let windows = windows(&request).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].request.state, "SOURCE 0: Synthetic action.");
        assert_eq!(windows[1].request.state, "SOURCE 0: Synthetic decision.");
        assert_eq!(windows[0].original_indices, (0..8).collect::<Vec<_>>());
        assert_eq!(windows[1].original_indices, (8..16).collect::<Vec<_>>());
        assert_eq!(windows[0].request.questions[0].id, "action");
        assert_eq!(
            windows[0].request.questions[0].instructions,
            request.questions[0].instructions
        );
    }
    #[test]
    fn missing_windows_components_wrong_namespace_and_remote_prefix_are_refused() {
        for change in 0..5 {
            let mut request = request();
            match change {
                0 => {
                    request.questions.pop();
                }
                1 => request.questions[0].id = "w8_action".into(),
                2 => {
                    request.questions[0].instructions =
                        "Use only WINDOW w0 in the untrusted map.".into()
                }
                3 => request.state = request.state.replace(FORMAT, "untrusted-window-map-v2"),
                _ => request.state = request.state.replace("\"w1\"", "\"w2\""),
            }
            assert!(windows(&request).is_err(), "change {change}");
        }
    }

    #[test]
    fn recommended_question_sequences_never_share_questions_or_sources() {
        let request = request();
        let sequences = questions(&request).unwrap();
        assert_eq!(sequences.len(), 16);
        for (index, sequence) in sequences.iter().enumerate() {
            assert_eq!(sequence.request.questions.len(), 1);
            assert_eq!(sequence.original_indices, vec![index]);
            assert_eq!(
                sequence.request.state,
                if index < 8 {
                    "SOURCE 0: Synthetic action."
                } else {
                    "SOURCE 0: Synthetic decision."
                }
            );
            assert_eq!(sequence.request.questions[0].id, COMPONENTS[index % 8]);
        }
    }
}
