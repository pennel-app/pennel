//! Local typing decisions without an engine, storage, network, or content logging.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "ffi")]
use std::ffi::{CString, c_char};

pub const MAX_REQUEST_BYTES: usize = 1_048_576;
const MAX_WORD_BYTES: usize = 128;
const MAX_WORD_CHARS: usize = 64;
const MAX_VOCABULARY: usize = 4096;
const MAX_PERSONAL_WORDS: usize = 2048;
const MAX_POINTS: usize = 256;
const MAX_KEYS: usize = 64;
const SWIPE_SAMPLES: usize = 32;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CandidateKind {
    #[default]
    Completion,
    Correction,
    Prediction,
    Snippet,
    Swipe,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlatformCandidate {
    pub text: String,
    pub language: String,
    #[serde(default = "correction_kind")]
    pub kind: CandidateKind,
}

fn correction_kind() -> CandidateKind {
    CandidateKind::Correction
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Word {
    pub text: String,
    pub language: String,
    #[serde(default = "one")]
    pub frequency: u32,
}

fn one() -> u32 {
    1
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NextWord {
    pub previous: String,
    pub text: String,
    pub count: u32,
    pub language: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snippet {
    pub trigger: String,
    pub replacement: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub time_ms: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Key {
    pub key: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TypingRequest {
    pub token: String,
    pub enabled_languages: Vec<String>,
    pub platform_candidates: Vec<PlatformCandidate>,
    pub valid_in_any_language: bool,
    pub personal_words: Vec<String>,
    pub vocabulary: Vec<Word>,
    pub at_word_boundary: bool,
    pub allow_automatic_correction: bool,
    pub rejected_candidates: Vec<String>,
    pub previous_token: Option<String>,
    pub learned_next_words: Vec<NextWord>,
    pub snippets: Vec<Snippet>,
    pub allow_snippet_expansion: bool,
    pub touches: Vec<Point>,
    pub keys: Vec<Key>,
    pub max_candidates: usize,
}

impl Default for TypingRequest {
    fn default() -> Self {
        Self {
            token: String::new(),
            enabled_languages: default_languages(),
            platform_candidates: Vec::new(),
            valid_in_any_language: false,
            personal_words: Vec::new(),
            vocabulary: Vec::new(),
            at_word_boundary: false,
            allow_automatic_correction: false,
            rejected_candidates: Vec::new(),
            previous_token: None,
            learned_next_words: Vec::new(),
            snippets: Vec::new(),
            allow_snippet_expansion: false,
            touches: Vec::new(),
            keys: Vec::new(),
            max_candidates: 3,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct SwipeRequest {
    pub path: Vec<Point>,
    pub keys: Vec<Key>,
    pub enabled_languages: Vec<String>,
    pub vocabulary: Vec<Word>,
    pub personal_words: Vec<String>,
    pub max_candidates: usize,
}

impl Default for SwipeRequest {
    fn default() -> Self {
        Self {
            path: Vec::new(),
            keys: Vec::new(),
            enabled_languages: default_languages(),
            vocabulary: Vec::new(),
            personal_words: Vec::new(),
            max_candidates: 3,
        }
    }
}

fn default_languages() -> Vec<String> {
    vec!["en".into(), "nl".into()]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub text: String,
    pub kind: CandidateKind,
    /// A relative ranking utility, never a calibrated probability.
    pub score: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypingResponse {
    pub candidates: Vec<Candidate>,
    pub automatic_replacement: Option<String>,
    pub replacement_kind: Option<CandidateKind>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TypingError {
    InvalidRequest,
    UnsupportedLanguage,
    RequestTooLarge,
}

impl TypingError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::UnsupportedLanguage => "unsupported_language",
            Self::RequestTooLarge => "request_too_large",
        }
    }
}

fn normal(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|character| if character == '’' { '\'' } else { character })
        .collect()
}

fn valid_word(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_WORD_BYTES
        && text.chars().count() <= MAX_WORD_CHARS
        && !text
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn valid_languages(languages: &[String]) -> Result<(), TypingError> {
    if languages.is_empty() || languages.len() > 2 {
        return Err(TypingError::InvalidRequest);
    }
    if languages
        .iter()
        .any(|language| !matches!(language.as_str(), "en" | "nl"))
    {
        return Err(TypingError::UnsupportedLanguage);
    }
    if languages.iter().collect::<BTreeSet<_>>().len() != languages.len() {
        return Err(TypingError::InvalidRequest);
    }
    Ok(())
}

fn language_enabled(language: &str, enabled: &[String]) -> bool {
    enabled.iter().any(|value| value == language)
}

fn validate_vocabulary(words: &[Word], personal: &[String]) -> Result<(), TypingError> {
    if words.len() > MAX_VOCABULARY || personal.len() > MAX_PERSONAL_WORDS {
        return Err(TypingError::RequestTooLarge);
    }
    if words
        .iter()
        .any(|word| !valid_word(&word.text) || !matches!(word.language.as_str(), "en" | "nl"))
        || personal.iter().any(|word| !valid_word(word))
    {
        return Err(TypingError::InvalidRequest);
    }
    Ok(())
}

fn validate_geometry(points: &[Point], keys: &[Key]) -> Result<(), TypingError> {
    if points.len() > MAX_POINTS || keys.len() > MAX_KEYS {
        return Err(TypingError::RequestTooLarge);
    }
    let coordinate = |value: f64| value.is_finite() && value.abs() <= 1_000_000.0;
    let mut previous_time = None;
    for point in points {
        if !coordinate(point.x) || !coordinate(point.y) {
            return Err(TypingError::InvalidRequest);
        }
        if let Some(time) = point.time_ms {
            if !time.is_finite()
                || !(0.0..=600_000.0).contains(&time)
                || previous_time.is_some_and(|previous| time < previous)
            {
                return Err(TypingError::InvalidRequest);
            }
            previous_time = Some(time);
        }
    }
    let mut seen = BTreeSet::new();
    for key in keys {
        let normalized = normal(&key.key);
        if normalized.chars().count() != 1
            || !normalized.chars().all(char::is_alphabetic)
            || !coordinate(key.x)
            || !coordinate(key.y)
            || !seen.insert(normalized)
        {
            return Err(TypingError::InvalidRequest);
        }
    }
    for (index, left) in keys.iter().enumerate() {
        if keys[index + 1..]
            .iter()
            .any(|right| (left.x - right.x).hypot(left.y - right.y) < 1e-9)
        {
            return Err(TypingError::InvalidRequest);
        }
    }
    Ok(())
}

fn validate_request(request: &TypingRequest) -> Result<(), TypingError> {
    valid_languages(&request.enabled_languages)?;
    validate_vocabulary(&request.vocabulary, &request.personal_words)?;
    validate_geometry(&request.touches, &request.keys)?;
    if request.token.len() > MAX_WORD_BYTES
        || request.token.chars().count() > MAX_WORD_CHARS
        || request.platform_candidates.len() > 64
        || request.rejected_candidates.len() > 64
        || request.learned_next_words.len() > 512
        || request.snippets.len() > 128
    {
        return Err(TypingError::RequestTooLarge);
    }
    if request.max_candidates == 0
        || request.max_candidates > 8
        || request
            .token
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || request
            .previous_token
            .as_deref()
            .is_some_and(|token| !valid_word(token))
        || request.platform_candidates.iter().any(|candidate| {
            !valid_word(&candidate.text)
                || !matches!(candidate.language.as_str(), "en" | "nl")
                || !matches!(
                    candidate.kind,
                    CandidateKind::Completion | CandidateKind::Correction
                )
        })
        || request
            .rejected_candidates
            .iter()
            .any(|text| text.is_empty() || text.len() > 2048)
        || request.learned_next_words.iter().any(|word| {
            !valid_word(&word.previous)
                || !valid_word(&word.text)
                || !matches!(word.language.as_str(), "en" | "nl")
        })
        || request.snippets.iter().any(|snippet| {
            !valid_word(&snippet.trigger)
                || snippet.replacement.is_empty()
                || snippet.replacement.len() > 2048
                || snippet
                    .replacement
                    .chars()
                    .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        })
    {
        return Err(TypingError::InvalidRequest);
    }
    Ok(())
}

#[derive(Clone)]
struct RankedWord {
    text: String,
    frequency: u32,
    personal: bool,
}

// Small authored fallback vocabulary; these weights express ranking priorities, not corpus statistics.
const ENGLISH: &str = "the to and a of in is it you that for on with this have be are not but we can at from as your all will one about what when there do if an they my so or me just was out up how good get time more know like no thanks thank please hello yes today tomorrow now see need would could should some our their who where which why want work well also then them really think very much make back only still here been had has i am done home message meeting call send go going come help happy great new next later morning evening sorry love have nice see soon regards hello world because maybe receive definitely weird something someone before after again over under into other people first last never always right left let use using ready address email number phone name friend family tomorrow yesterday together available appointment question answer afternoon information important project review update pending pencil keyboard typing voice english dutch";
const DUTCH: &str = "de het een en van ik je dat die in is op te niet voor met zijn aan er maar om ook als dit dan nog wat we bij naar mijn hij haar ze zo door kan heb geen wel uit wordt deze aan meer tot al hebben was ben hoe of over moet mij wie dus ons zou voor jou goed dank dankje bedankt graag hallo ja nee vandaag morgen nu straks misschien wanneer waarom waar welke kunnen willen gaan komen doen zien weten even iets niets iemand niemand veel heel samen thuis werk bericht afspraak vergadering bellen sturen tijd vraag antwoord ochtend middag avond gisteren alsjeblieft eigenlijk onmiddellijk natuurlijk Nederlands Engels groeten vriendelijke groetjes fijne dag leuk prettig zeker inderdaad hebben worden waren geweest doen gedaan komt gaat terug alvast eens weer maken alleen altijd nooit lezen schrijven spreken toetsenbord stem woord woorden typen naam mensen andere nieuwe eerste laatste beste lieve huis familie vriend volgende later beschikbaar ontvangen belangrijk informatie project bespreken overzicht rekening nummer telefoon adres e-mail scherm instellingen woordenboek correctie suggestie opnemen bewaren aanpassen bedankt pendel";

fn word_pool(languages: &[String], vocabulary: &[Word], personal: &[String]) -> Vec<RankedWord> {
    let mut pool = BTreeMap::<String, RankedWord>::new();
    for (language, text) in [("en", ENGLISH), ("nl", DUTCH)] {
        if !language_enabled(language, languages) {
            continue;
        }
        for (index, word) in text.split_whitespace().enumerate() {
            pool.entry(normal(word)).or_insert_with(|| RankedWord {
                text: word.into(),
                frequency: 300_u32.saturating_sub(index as u32).max(1),
                personal: false,
            });
        }
    }
    for word in vocabulary
        .iter()
        .filter(|word| language_enabled(&word.language, languages))
    {
        let key = normal(&word.text);
        let entry = pool.entry(key).or_insert_with(|| RankedWord {
            text: word.text.clone(),
            frequency: 0,
            personal: false,
        });
        if word.frequency >= entry.frequency {
            entry.text = word.text.clone();
            entry.frequency = word.frequency;
        }
    }
    for word in personal {
        pool.insert(
            normal(word),
            RankedWord {
                text: word.clone(),
                frequency: 1000,
                personal: true,
            },
        );
    }
    pool.into_values().collect()
}

fn word_score(frequency: u32) -> f64 {
    f64::from(frequency.max(1)).ln_1p()
}

fn insert_candidate(
    candidates: &mut BTreeMap<String, Candidate>,
    candidate: Candidate,
    rejected: &BTreeSet<String>,
) {
    let key = normal(&candidate.text);
    if rejected.contains(&key) {
        return;
    }
    if candidates
        .get(&key)
        .is_none_or(|existing| candidate.score > existing.score)
    {
        candidates.insert(key, candidate);
    }
}

fn ranked(candidates: BTreeMap<String, Candidate>, limit: usize) -> Vec<Candidate> {
    let mut values: Vec<_> = candidates.into_values().collect();
    values.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.text.cmp(&right.text))
    });
    values.truncate(limit);
    values
}

fn protected(token: &str) -> bool {
    token
        .chars()
        .any(|character| !character.is_alphabetic() || character.is_uppercase())
}

fn known_typo(token: &str, languages: &[String]) -> Option<&'static str> {
    let entries = [
        ("en", "teh", "the"),
        ("en", "hte", "the"),
        ("en", "adn", "and"),
        ("en", "taht", "that"),
        ("en", "thsi", "this"),
        ("en", "becuase", "because"),
        ("en", "recieve", "receive"),
        ("en", "definately", "definitely"),
        ("en", "tomorow", "tomorrow"),
        ("en", "tommorow", "tomorrow"),
        ("nl", "mischien", "misschien"),
        ("nl", "meschien", "misschien"),
        ("nl", "waneer", "wanneer"),
        ("nl", "wanneeer", "wanneer"),
        ("nl", "eigelijk", "eigenlijk"),
        ("nl", "onmiddelijk", "onmiddellijk"),
        ("nl", "natuulijk", "natuurlijk"),
        ("nl", "bednakt", "bedankt"),
    ];
    entries
        .into_iter()
        .find_map(|(language, original, replacement)| {
            (original == token && language_enabled(language, languages)).then_some(replacement)
        })
}

fn edit_distance(left: &str, right: &str, cutoff: usize) -> usize {
    let left: Vec<_> = left.chars().collect();
    let right: Vec<_> = right.chars().collect();
    if left.len().abs_diff(right.len()) > cutoff {
        return cutoff + 1;
    }
    let mut previous_previous = vec![0; right.len() + 1];
    let mut previous: Vec<_> = (0..=right.len()).collect();
    for (i, a) in left.iter().enumerate() {
        let mut current = vec![i + 1; right.len() + 1];
        for (j, b) in right.iter().enumerate() {
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + usize::from(a != b));
            if i > 0 && j > 0 && *a == right[j - 1] && left[i - 1] == *b {
                current[j + 1] = current[j + 1].min(previous_previous[j - 1] + 1);
            }
        }
        previous_previous = previous;
        previous = current;
    }
    previous[right.len()]
}

fn touch_adjustment(word: &str, touches: &[Point], keys: &[Key]) -> f64 {
    if touches.is_empty() || touches.len() != word.chars().count() {
        return 0.0;
    }
    let Some(spacing) = key_spacing(keys) else {
        return 0.0;
    };
    let mut cost = 0.0;
    for (character, point) in normal(word).chars().zip(touches) {
        let Some(key) = keys
            .iter()
            .find(|key| normal(&key.key) == character.to_string())
        else {
            return 0.0;
        };
        cost += ((point.x - key.x).hypot(point.y - key.y) / spacing).min(3.0);
    }
    -(cost / touches.len() as f64) * 8.0
}

pub fn suggest(request: &TypingRequest) -> Result<TypingResponse, TypingError> {
    validate_request(request)?;
    let token = normal(&request.token);
    let rejected: BTreeSet<_> = request
        .rejected_candidates
        .iter()
        .map(|value| normal(value))
        .collect();
    let pool = word_pool(
        &request.enabled_languages,
        &request.vocabulary,
        &request.personal_words,
    );
    let known_valid =
        request.valid_in_any_language || pool.iter().any(|word| normal(&word.text) == token);
    let mut candidates = BTreeMap::new();
    let mut response = TypingResponse::default();

    let snippet_matches: Vec<_> = request
        .snippets
        .iter()
        .filter(|snippet| snippet.trigger == request.token)
        .collect();
    for snippet in &snippet_matches {
        insert_candidate(
            &mut candidates,
            Candidate {
                text: snippet.replacement.clone(),
                kind: CandidateKind::Snippet,
                score: 1500.0,
            },
            &rejected,
        );
    }
    if request.allow_snippet_expansion
        && request.at_word_boundary
        && snippet_matches.len() == 1
        && !rejected.contains(&normal(&snippet_matches[0].replacement))
    {
        response.automatic_replacement = Some(snippet_matches[0].replacement.clone());
        response.replacement_kind = Some(CandidateKind::Snippet);
    }

    if token.is_empty() {
        if let Some(previous) = request.previous_token.as_deref() {
            let previous = normal(previous);
            for word in request.learned_next_words.iter().filter(|word| {
                normal(&word.previous) == previous
                    && language_enabled(&word.language, &request.enabled_languages)
            }) {
                insert_candidate(
                    &mut candidates,
                    Candidate {
                        text: word.text.clone(),
                        kind: CandidateKind::Prediction,
                        score: 100.0 + word_score(word.count),
                    },
                    &rejected,
                );
            }
        }
        response.candidates = ranked(candidates, request.max_candidates);
        return Ok(response);
    }

    let is_protected = protected(&request.token);
    for word in &pool {
        let normalized = normal(&word.text);
        if normalized == token {
            continue;
        }
        if normalized.starts_with(&token) {
            let score =
                180.0 + word_score(word.frequency) + if word.personal { 1000.0 } else { 0.0 };
            insert_candidate(
                &mut candidates,
                Candidate {
                    text: word.text.clone(),
                    kind: CandidateKind::Completion,
                    score,
                },
                &rejected,
            );
        } else if !is_protected && !known_valid && token.chars().count() >= 3 {
            let distance = edit_distance(&token, &normalized, 2);
            if distance <= 2 {
                let score = 100.0 - distance as f64 * 20.0
                    + word_score(word.frequency)
                    + touch_adjustment(&word.text, &request.touches, &request.keys);
                insert_candidate(
                    &mut candidates,
                    Candidate {
                        text: word.text.clone(),
                        kind: CandidateKind::Correction,
                        score,
                    },
                    &rejected,
                );
            }
        }
    }
    for (index, candidate) in request.platform_candidates.iter().enumerate() {
        if !language_enabled(&candidate.language, &request.enabled_languages)
            || normal(&candidate.text) == token
        {
            continue;
        }
        let normalized = normal(&candidate.text);
        if candidate.kind == CandidateKind::Completion && !normalized.starts_with(&token) {
            continue;
        }
        if candidate.kind == CandidateKind::Correction
            && (is_protected || known_valid || edit_distance(&token, &normalized, 2) > 2)
        {
            continue;
        }
        let base = if candidate.kind == CandidateKind::Completion {
            200.0
        } else {
            130.0
        };
        insert_candidate(
            &mut candidates,
            Candidate {
                text: candidate.text.clone(),
                kind: candidate.kind,
                score: base - index as f64 * 0.5
                    + touch_adjustment(&candidate.text, &request.touches, &request.keys),
            },
            &rejected,
        );
    }
    if !is_protected
        && !known_valid
        && let Some(replacement) = known_typo(&token, &request.enabled_languages)
            .filter(|replacement| !rejected.contains(*replacement))
    {
        insert_candidate(
            &mut candidates,
            Candidate {
                text: replacement.into(),
                kind: CandidateKind::Correction,
                score: 500.0,
            },
            &rejected,
        );
        // Automatic edits are restricted to authored typo mappings. Ranking alone is not confidence.
        if request.allow_automatic_correction
            && request.at_word_boundary
            && response.automatic_replacement.is_none()
        {
            response.automatic_replacement = Some(replacement.into());
            response.replacement_kind = Some(CandidateKind::Correction);
        }
    }
    response.candidates = ranked(candidates, request.max_candidates);
    Ok(response)
}

fn key_spacing(keys: &[Key]) -> Option<f64> {
    let mut minimum = f64::INFINITY;
    for (index, left) in keys.iter().enumerate() {
        for right in &keys[index + 1..] {
            minimum = minimum.min((left.x - right.x).hypot(left.y - right.y));
        }
    }
    (minimum.is_finite() && minimum > 1e-9).then_some(minimum)
}

fn resample(points: &[Point]) -> Option<Vec<Point>> {
    let first = *points.first()?;
    let mut lengths = vec![0.0];
    for pair in points.windows(2) {
        lengths.push(
            lengths.last().copied().unwrap_or(0.0)
                + (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y),
        );
    }
    let total = *lengths.last()?;
    if total < 1e-9 {
        return None;
    }
    let mut sampled = Vec::with_capacity(SWIPE_SAMPLES);
    let mut segment = 1;
    for index in 0..SWIPE_SAMPLES {
        let target = total * index as f64 / (SWIPE_SAMPLES - 1) as f64;
        while segment < lengths.len() - 1 && lengths[segment] < target {
            segment += 1;
        }
        let previous = segment.saturating_sub(1);
        let width = lengths[segment] - lengths[previous];
        let amount = if width > 0.0 {
            (target - lengths[previous]) / width
        } else {
            0.0
        };
        let a = points.get(previous).copied().unwrap_or(first);
        let b = points.get(segment).copied().unwrap_or(a);
        sampled.push(Point {
            x: a.x + (b.x - a.x) * amount,
            y: a.y + (b.y - a.y) * amount,
            time_ms: None,
        });
    }
    Some(sampled)
}

pub fn decode_swipe(request: &SwipeRequest) -> Result<TypingResponse, TypingError> {
    valid_languages(&request.enabled_languages)?;
    validate_vocabulary(&request.vocabulary, &request.personal_words)?;
    validate_geometry(&request.path, &request.keys)?;
    if request.path.len() < 2
        || request.keys.len() < 2
        || !(1..=8).contains(&request.max_candidates)
    {
        return Err(TypingError::InvalidRequest);
    }
    let spacing = key_spacing(&request.keys).ok_or(TypingError::InvalidRequest)?;
    let path = resample(&request.path).ok_or(TypingError::InvalidRequest)?;
    let geometry: BTreeMap<_, _> = request
        .keys
        .iter()
        .filter_map(|key| {
            normal(&key.key).chars().next().map(|character| {
                (
                    character,
                    Point {
                        x: key.x,
                        y: key.y,
                        time_ms: None,
                    },
                )
            })
        })
        .collect();
    let pool = word_pool(
        &request.enabled_languages,
        &request.vocabulary,
        &request.personal_words,
    );
    let mut candidates = BTreeMap::new();
    let rejected = BTreeSet::new();
    for word in pool {
        let mut ideal = Vec::new();
        let mut previous = None;
        let mut supported = true;
        for character in normal(&word.text).chars() {
            if previous == Some(character) {
                continue;
            }
            previous = Some(character);
            let Some(key) = geometry.get(&character) else {
                supported = false;
                break;
            };
            ideal.push(*key);
        }
        if !supported {
            continue;
        }
        let Some(ideal) = resample(&ideal) else {
            continue;
        };
        let distance = |a: Point, b: Point| (a.x - b.x).hypot(a.y - b.y) / spacing;
        let start = distance(path[0], ideal[0]);
        let end = distance(path[SWIPE_SAMPLES - 1], ideal[SWIPE_SAMPLES - 1]);
        if start > 1.25 || end > 1.25 {
            continue;
        }
        let shape = path
            .iter()
            .zip(&ideal)
            .map(|(actual, expected)| distance(*actual, *expected).powi(2))
            .sum::<f64>()
            / SWIPE_SAMPLES as f64;
        if shape > 4.0 {
            continue;
        }
        let score = 500.0 - shape * 100.0 - (start + end) * 50.0
            + word_score(word.frequency)
            + if word.personal { 3.0 } else { 0.0 };
        insert_candidate(
            &mut candidates,
            Candidate {
                text: word.text,
                kind: CandidateKind::Swipe,
                score,
            },
            &rejected,
        );
    }
    Ok(TypingResponse {
        candidates: ranked(candidates, request.max_candidates),
        ..TypingResponse::default()
    })
}

#[cfg(feature = "ffi")]
fn encode_response(result: Result<TypingResponse, TypingError>) -> *mut c_char {
    let response = match result {
        Ok(response) => response,
        Err(error) => TypingResponse {
            error: Some(error.code().into()),
            ..TypingResponse::default()
        },
    };
    let json = serde_json::to_vec(&response).unwrap_or_else(|_| b"{\"candidates\":[],\"automaticReplacement\":null,\"replacementKind\":null,\"error\":\"encoding_failed\"}".to_vec());
    CString::new(json)
        .expect("JSON serialization escapes NUL characters")
        .into_raw()
}

#[cfg(feature = "ffi")]
unsafe fn request_bytes<'a>(bytes: *const u8, length: usize) -> Result<&'a [u8], TypingError> {
    if length > MAX_REQUEST_BYTES {
        return Err(TypingError::RequestTooLarge);
    }
    if bytes.is_null() || length == 0 {
        return Err(TypingError::InvalidRequest);
    }
    // The C contract requires a readable allocation of at least `length` bytes for this call.
    Ok(unsafe { std::slice::from_raw_parts(bytes, length) })
}

/// Evaluate a UTF-8 JSON TypingRequest. The returned UTF-8 C string is owned by Rust.
///
/// # Safety
/// `bytes` must be readable for `length` bytes for the duration of this call. Release the returned
/// pointer exactly once using `pennel_typing_free_response`, never the caller's allocator.
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pennel_typing_request_json(
    bytes: *const u8,
    length: usize,
) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        let bytes = unsafe { request_bytes(bytes, length) }?;
        let request: TypingRequest =
            serde_json::from_slice(bytes).map_err(|_| TypingError::InvalidRequest)?;
        suggest(&request)
    })
    .unwrap_or(Err(TypingError::InvalidRequest));
    encode_response(result)
}

/// Evaluate a UTF-8 JSON SwipeRequest with all coordinates in the same coordinate space.
///
/// # Safety
/// The readable-buffer and returned-allocation requirements are the same as
/// `pennel_typing_request_json`.
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pennel_typing_swipe_json(bytes: *const u8, length: usize) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        let bytes = unsafe { request_bytes(bytes, length) }?;
        let request: SwipeRequest =
            serde_json::from_slice(bytes).map_err(|_| TypingError::InvalidRequest)?;
        decode_swipe(&request)
    })
    .unwrap_or(Err(TypingError::InvalidRequest));
    encode_response(result)
}

/// Release a response allocated by either request function. NULL is accepted.
///
/// # Safety
/// A nonnull pointer must be an outstanding allocation returned by this library, passed exactly once.
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pennel_typing_free_response(response: *mut c_char) {
    if !response.is_null() {
        drop(unsafe { CString::from_raw(response) });
    }
}
