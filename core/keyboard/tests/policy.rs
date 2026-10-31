use pennel_typing::*;
#[cfg(feature = "ffi")]
use std::ffi::CStr;

fn correction(text: &str, language: &str) -> PlatformCandidate {
    PlatformCandidate {
        text: text.into(),
        language: language.into(),
        kind: CandidateKind::Correction,
    }
}

fn boundary(token: &str) -> TypingRequest {
    TypingRequest {
        token: token.into(),
        at_word_boundary: true,
        allow_automatic_correction: true,
        ..TypingRequest::default()
    }
}

#[test]
fn explicit_english_and_dutch_typos_correct_locally() {
    let mut english = boundary("teh");
    english.platform_candidates.push(correction("the", "en"));
    assert_eq!(
        suggest(&english).unwrap().automatic_replacement.as_deref(),
        Some("the")
    );
    let mut dutch = boundary("mischien");
    dutch
        .platform_candidates
        .push(correction("misschien", "nl"));
    assert_eq!(
        suggest(&dutch).unwrap().automatic_replacement.as_deref(),
        Some("misschien")
    );
}

#[test]
fn validity_in_either_language_protects_mixed_language_typing() {
    for token in ["of", "is", "die", "we", "kind", "brief"] {
        let mut request = boundary(token);
        request.valid_in_any_language = true;
        request
            .platform_candidates
            .push(correction("different", "en"));
        assert!(suggest(&request).unwrap().automatic_replacement.is_none());
    }
    let mut request = boundary("teh");
    request.valid_in_any_language = true;
    assert!(suggest(&request).unwrap().automatic_replacement.is_none());
}

#[test]
fn names_numbers_addresses_and_unknown_tokens_never_autocorrect() {
    for token in [
        "Lander",
        "britt",
        "Pennel",
        "1234",
        "12,50",
        "name@example.com",
        "https://pennel.app",
        "snake_case",
        "a1b2",
        "teh.org",
    ] {
        let mut request = boundary(token);
        request.platform_candidates.push(correction("the", "en"));
        assert!(
            suggest(&request).unwrap().automatic_replacement.is_none(),
            "{token}"
        );
    }
}

#[test]
fn explicit_personal_vocabulary_and_rejected_replacement_win() {
    let mut request = boundary("teh");
    request.personal_words.push("teh".into());
    assert!(suggest(&request).unwrap().automatic_replacement.is_none());
    request.personal_words.clear();
    request.rejected_candidates.push("the".into());
    let response = suggest(&request).unwrap();
    assert!(response.automatic_replacement.is_none());
    assert!(
        response
            .candidates
            .iter()
            .all(|candidate| candidate.text != "the")
    );
}

#[test]
fn ambiguous_platform_guesses_are_suggestions_only() {
    let mut request = boundary("gello");
    request.platform_candidates = vec![correction("hello", "en"), correction("jello", "en")];
    let response = suggest(&request).unwrap();
    assert!(response.automatic_replacement.is_none());
    assert!(
        response
            .candidates
            .iter()
            .any(|candidate| candidate.text == "hello")
    );
}

#[test]
fn language_filtering_and_word_boundary_are_enforced() {
    let mut request = boundary("teh");
    request.enabled_languages = vec!["nl".into()];
    request.platform_candidates.push(correction("the", "en"));
    assert!(suggest(&request).unwrap().automatic_replacement.is_none());
    request.enabled_languages = vec!["en".into()];
    request.at_word_boundary = false;
    assert!(suggest(&request).unwrap().automatic_replacement.is_none());
}

#[test]
fn completions_use_personal_vocabulary_and_frequency_not_alphabetical_order() {
    let request = TypingRequest {
        token: "pen".into(),
        personal_words: vec!["Pennel".into()],
        vocabulary: vec![
            Word {
                text: "pencil".into(),
                language: "en".into(),
                frequency: 1,
            },
            Word {
                text: "pending".into(),
                language: "en".into(),
                frequency: 500,
            },
        ],
        ..TypingRequest::default()
    };
    let response = suggest(&request).unwrap();
    assert_eq!(response.candidates[0].text, "Pennel");
    let pending = response
        .candidates
        .iter()
        .position(|candidate| candidate.text == "pending")
        .unwrap();
    let pencil = response
        .candidates
        .iter()
        .position(|candidate| candidate.text == "pencil")
        .unwrap();
    assert!(pending < pencil);
}

#[test]
fn learned_next_word_counts_and_snippets_are_local_explicit_inputs() {
    let request = TypingRequest {
        previous_token: Some("tot".into()),
        learned_next_words: vec![
            NextWord {
                previous: "tot".into(),
                text: "morgen".into(),
                count: 30,
                language: "nl".into(),
            },
            NextWord {
                previous: "tot".into(),
                text: "straks".into(),
                count: 2,
                language: "nl".into(),
            },
        ],
        ..TypingRequest::default()
    };
    let response = suggest(&request).unwrap();
    assert_eq!(response.candidates[0].text, "morgen");
    let mut request = boundary(";sig");
    request.allow_snippet_expansion = true;
    request.snippets = vec![Snippet {
        trigger: ";sig".into(),
        replacement: "Kind regards,\nLander".into(),
    }];
    assert_eq!(
        suggest(&request).unwrap().automatic_replacement.as_deref(),
        Some("Kind regards,\nLander")
    );
    request.allow_snippet_expansion = false;
    assert!(suggest(&request).unwrap().automatic_replacement.is_none());
}

fn keyboard(rows: &[&str]) -> Vec<Key> {
    rows.iter()
        .enumerate()
        .flat_map(|(y, row)| {
            row.chars().enumerate().map(move |(x, key)| Key {
                key: key.to_string(),
                x: x as f64,
                y: y as f64,
            })
        })
        .collect()
}

fn path(word: &str, keys: &[Key]) -> Vec<Point> {
    word.chars()
        .map(|character| {
            let key = keys
                .iter()
                .find(|key| key.key == character.to_string())
                .unwrap();
            Point {
                x: key.x,
                y: key.y,
                time_ms: None,
            }
        })
        .collect()
}

#[test]
fn swipe_decodes_geometry_for_qwerty_and_azerty_without_network() {
    for rows in [
        ["qwertyuiop", "asdfghjkl", "zxcvbnm"],
        ["azertyuiop", "qsdfghjklm", "wxcvbn"],
    ] {
        let keys = keyboard(&rows);
        let request = SwipeRequest {
            path: path("hello", &keys),
            keys,
            vocabulary: vec![Word {
                text: "hello".into(),
                language: "en".into(),
                frequency: 1000,
            }],
            ..SwipeRequest::default()
        };
        let response = decode_swipe(&request).unwrap();
        assert_eq!(response.candidates[0].text, "hello");
        assert_eq!(response.candidates[0].kind, CandidateKind::Swipe);
        assert!(response.automatic_replacement.is_none());
    }
}

#[test]
fn malformed_and_overlong_inputs_are_rejected_without_truncating_intent() {
    let request = TypingRequest {
        token: "a".repeat(129),
        ..TypingRequest::default()
    };
    assert!(suggest(&request).is_err());
    let request = TypingRequest {
        enabled_languages: vec!["unsupported".into()],
        ..TypingRequest::default()
    };
    assert!(suggest(&request).is_err());
    let request = SwipeRequest {
        path: vec![Point {
            x: f64::NAN,
            y: 0.0,
            time_ms: None,
        }],
        ..SwipeRequest::default()
    };
    assert!(decode_swipe(&request).is_err());
}

#[test]
fn default_swipe_vocabulary_covers_both_languages_without_personal_data() {
    let keys = keyboard(&["qwertyuiop", "asdfghjkl", "zxcvbnm"]);
    for (word, language) in [("hello", "en"), ("morgen", "nl")] {
        let request = SwipeRequest {
            path: path(word, &keys),
            keys: keys.clone(),
            enabled_languages: vec![language.into()],
            ..SwipeRequest::default()
        };
        assert_eq!(decode_swipe(&request).unwrap().candidates[0].text, word);
    }
}

#[test]
fn ambiguous_snippet_and_language_scoped_predictions_cannot_expand_unexpectedly() {
    let mut request = boundary(";sig");
    request.allow_snippet_expansion = true;
    request.snippets = vec![
        Snippet {
            trigger: ";sig".into(),
            replacement: "First".into(),
        },
        Snippet {
            trigger: ";sig".into(),
            replacement: "Second".into(),
        },
    ];
    assert!(suggest(&request).unwrap().automatic_replacement.is_none());
    let request = TypingRequest {
        previous_token: Some("tot".into()),
        enabled_languages: vec!["en".into()],
        learned_next_words: vec![NextWord {
            previous: "tot".into(),
            text: "morgen".into(),
            count: 99,
            language: "nl".into(),
        }],
        ..TypingRequest::default()
    };
    assert!(suggest(&request).unwrap().candidates.is_empty());
}

#[test]
fn invalid_geometry_and_excessive_arrays_fail_closed() {
    let request = TypingRequest {
        personal_words: vec!["valid".into(); 2049],
        ..TypingRequest::default()
    };
    assert_eq!(suggest(&request).unwrap_err(), TypingError::RequestTooLarge);
    let mut request = SwipeRequest {
        path: vec![
            Point {
                x: 0.0,
                y: 0.0,
                time_ms: None
            };
            2
        ],
        keys: keyboard(&["ab"]),
        ..SwipeRequest::default()
    };
    assert!(decode_swipe(&request).is_err());
    request.path[1].x = 1.0;
    request.keys[1].x = 0.0;
    assert!(decode_swipe(&request).is_err());
    request.keys = keyboard(&["ab"]);
    request.path[0].time_ms = Some(100.0);
    request.path[1].time_ms = Some(50.0);
    assert!(decode_swipe(&request).is_err());
}

#[test]
fn unicode_completions_preserve_the_original_spelling() {
    let request = TypingRequest {
        token: "caf".into(),
        personal_words: vec!["café".into()],
        ..TypingRequest::default()
    };
    assert_eq!(suggest(&request).unwrap().candidates[0].text, "café");
    let mut request = boundary("mischien");
    request.allow_automatic_correction = false;
    let response = suggest(&request).unwrap();
    assert!(response.automatic_replacement.is_none());
    assert_eq!(response.candidates[0].text, "misschien");
}

#[test]
fn swipe_is_scale_independent_and_rejects_paths_outside_the_keys() {
    let keys = keyboard(&["qwertyuiop", "asdfghjkl", "zxcvbnm"]);
    let mut request = SwipeRequest {
        path: path("morgen", &keys),
        keys,
        enabled_languages: vec!["nl".into()],
        ..SwipeRequest::default()
    };
    for point in &mut request.path {
        point.x = 100.0 + point.x * 40.0;
        point.y = 200.0 + point.y * 40.0;
    }
    for key in &mut request.keys {
        key.x = 100.0 + key.x * 40.0;
        key.y = 200.0 + key.y * 40.0;
    }
    assert_eq!(decode_swipe(&request).unwrap().candidates[0].text, "morgen");
    for point in &mut request.path {
        point.y += 1000.0;
    }
    assert!(decode_swipe(&request).unwrap().candidates.is_empty());
}

#[test]
#[cfg(feature = "ffi")]
fn ffi_accepts_explicit_utf8_length_and_returns_owned_json() {
    let request = br#"{"token":"teh","atWordBoundary":true,"allowAutomaticCorrection":true}"#;
    unsafe {
        let response = pennel_typing_request_json(request.as_ptr(), request.len());
        assert!(!response.is_null());
        let json = CStr::from_ptr(response).to_str().unwrap();
        let response_value: TypingResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response_value.automatic_replacement.as_deref(), Some("the"));
        pennel_typing_free_response(response);
        pennel_typing_free_response(std::ptr::null_mut());
    }
}

#[test]
#[cfg(feature = "ffi")]
fn ffi_checks_length_and_encoding_before_parsing() {
    let invalid_utf8 = [0xffu8];
    unsafe {
        for (pointer, length) in [
            (std::ptr::null(), 10),
            (b"x".as_ptr(), MAX_REQUEST_BYTES + 1),
            (b"{".as_ptr(), 1),
            (invalid_utf8.as_ptr(), 1),
            (c"{}".as_ptr().cast::<u8>(), 3),
        ] {
            let response = pennel_typing_request_json(pointer, length);
            let response_value: TypingResponse =
                serde_json::from_slice(CStr::from_ptr(response).to_bytes()).unwrap();
            assert!(response_value.error.is_some());
            assert!(response_value.candidates.is_empty());
            pennel_typing_free_response(response);
        }
    }
}
