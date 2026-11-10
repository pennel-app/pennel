//! Content-free desktop capture levels. Time is supplied so throttling has deterministic tests.
use std::time::Duration;

pub type LevelCallback = std::sync::Arc<dyn Fn(u32, f32) + Send + Sync>;

pub struct LevelThrottle {
    session: String,
    last: Option<Duration>,
    sources: [Option<(f32, Duration)>; 2],
}
impl LevelThrottle {
    pub fn new(session: String) -> Self {
        Self {
            session,
            last: None,
            sources: [None, None],
        }
    }
    pub fn sample(
        &mut self,
        session: Option<&str>,
        recording: bool,
        source: u32,
        level: f32,
        now: Duration,
    ) -> Option<f32> {
        if !recording || session != Some(self.session.as_str()) || !level.is_finite() {
            return None;
        }
        let slot = self.sources.get_mut(source as usize)?;
        *slot = Some((level.clamp(0.0, 1.0), now));
        if self
            .last
            .is_some_and(|last| now.saturating_sub(last) < Duration::from_millis(50))
        {
            return None;
        }
        self.last = Some(now);
        // Keep the other source's latest short-lived peak even when its callback arrives second.
        Some(
            self.sources
                .iter()
                .flatten()
                .filter(|(_, received)| now.saturating_sub(*received) <= Duration::from_millis(250))
                .map(|(level, _)| *level)
                .fold(0.0, f32::max),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn either_source_drives_the_meter_even_when_the_silent_source_arrives_first() {
        for audible_source in 0..2 {
            let mut levels = LevelThrottle::new("capture".into());
            let silent_source = 1 - audible_source;
            assert_eq!(
                levels.sample(Some("capture"), true, silent_source, 0.0, Duration::ZERO),
                Some(0.0)
            );
            assert_eq!(
                levels.sample(
                    Some("capture"),
                    true,
                    audible_source,
                    0.8,
                    Duration::from_millis(1)
                ),
                None
            );
            for window in 1..4 {
                let now = Duration::from_millis(window * 50);
                assert_eq!(
                    levels.sample(Some("capture"), true, silent_source, 0.0, now),
                    Some(0.8)
                );
                assert_eq!(
                    levels.sample(
                        Some("capture"),
                        true,
                        audible_source,
                        0.8,
                        now + Duration::from_millis(1)
                    ),
                    None
                );
            }
            assert_eq!(
                levels.sample(
                    Some("capture"),
                    true,
                    silent_source,
                    0.0,
                    Duration::from_millis(500)
                ),
                Some(0.0)
            );
        }
    }
    #[test]
    fn caps_each_capture_at_twenty_events_per_second() {
        let mut levels = LevelThrottle::new("capture".into());
        let mut published = 0;
        for millisecond in 0..1_000 {
            published += usize::from(
                levels
                    .sample(
                        Some("capture"),
                        true,
                        0,
                        0.4,
                        Duration::from_millis(millisecond),
                    )
                    .is_some(),
            );
        }
        assert_eq!(published, 20);
    }
    #[test]
    fn rejects_stale_and_stopped_capture_without_spending_the_current_budget() {
        let mut levels = LevelThrottle::new("current".into());
        assert_eq!(
            levels.sample(Some("old"), true, 0, 1.0, Duration::ZERO),
            None
        );
        assert_eq!(
            levels.sample(Some("current"), false, 0, 1.0, Duration::ZERO),
            None
        );
        assert_eq!(levels.sample(None, true, 0, 1.0, Duration::ZERO), None);
        assert_eq!(
            levels.sample(Some("current"), true, 0, 0.25, Duration::ZERO),
            Some(0.25)
        );
        assert_eq!(
            levels.sample(Some("next"), true, 0, 1.0, Duration::from_millis(50)),
            None
        );
    }
    #[test]
    fn publishes_only_finite_normalized_levels_and_allows_the_fifty_ms_boundary() {
        let mut levels = LevelThrottle::new("capture".into());
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                levels.sample(Some("capture"), true, 0, value, Duration::ZERO),
                None
            );
        }
        assert_eq!(
            levels.sample(Some("capture"), true, 0, 4.0, Duration::ZERO),
            Some(1.0)
        );
        assert_eq!(
            levels.sample(Some("capture"), true, 0, 0.5, Duration::from_millis(49)),
            None
        );
        assert_eq!(
            levels.sample(Some("capture"), true, 0, -2.0, Duration::from_millis(50)),
            Some(0.0)
        );
    }
}
