use crate::speech::has_speech;

const MIN_SECONDS: u32 = 15;
const MAX_SECONDS: u32 = 30;
const FRAME_MS: u32 = 100;
const SILENCE_FRAMES: usize = 6;
const SILENCE_RMS: f32 = 0.01;

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub start_ms: u64,
    pub samples: Vec<f32>,
}

/// Splits a continuous track into 15–30 s pieces, preferring to cut in a pause so words aren't split.
pub struct Chunker {
    sample_rate: u32,
    buffer: Vec<f32>,
    buffer_start: u64,
    quiet: Vec<bool>,
}

impl Chunker {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            buffer: Vec::new(),
            buffer_start: 0,
            quiet: Vec::new(),
        }
    }

    fn frame_len(&self) -> usize {
        (self.sample_rate * FRAME_MS / 1000) as usize
    }

    pub fn push(&mut self, samples: &[f32]) -> Vec<Chunk> {
        self.buffer.extend_from_slice(samples);
        self.update_quiet();
        let mut chunks = Vec::new();
        while let Some(cut) = self.cut_point() {
            chunks.extend(self.take(cut));
        }
        chunks
    }

    pub fn flush(&mut self) -> Option<Chunk> {
        let len = self.buffer.len();
        if len == 0 {
            return None;
        }
        self.take(len)
    }

    pub(crate) fn current_start_ms(&self) -> u64 {
        self.buffer_start * 1000 / u64::from(self.sample_rate)
    }

    pub(crate) fn current_start_sample(&self) -> u64 {
        self.buffer_start
    }

    /// Finish the current audio before a device interruption without allocating the missing silence.
    pub fn skip_to(&mut self, sample_offset: u64) -> Option<Chunk> {
        let end = self.buffer_start + self.buffer.len() as u64;
        if sample_offset <= end {
            return None;
        }
        let last = self.flush();
        self.buffer_start = sample_offset;
        last
    }

    fn update_quiet(&mut self) {
        let frame = self.frame_len();
        while (self.quiet.len() + 1) * frame <= self.buffer.len() {
            let start = self.quiet.len() * frame;
            let window = &self.buffer[start..start + frame];
            let rms = (window.iter().map(|s| s * s).sum::<f32>() / frame as f32).sqrt();
            self.quiet.push(rms < SILENCE_RMS);
        }
    }

    fn cut_point(&self) -> Option<usize> {
        let frame = self.frame_len();
        let min = (self.sample_rate * MIN_SECONDS) as usize;
        let max = (self.sample_rate * MAX_SECONDS) as usize;
        let mut run = 0;
        for (i, quiet) in self.quiet.iter().enumerate() {
            let end = (i + 1) * frame;
            if end > max {
                break;
            }
            run = if *quiet { run + 1 } else { 0 };
            if run >= SILENCE_FRAMES && end >= min {
                return Some(end);
            }
        }
        (self.buffer.len() >= max).then_some(max)
    }

    fn take(&mut self, len: usize) -> Option<Chunk> {
        let samples: Vec<f32> = self.buffer.drain(..len).collect();
        let start_ms = self.buffer_start * 1000 / u64::from(self.sample_rate);
        self.buffer_start += len as u64;
        self.quiet.clear();
        self.update_quiet();
        has_speech(&samples, self.sample_rate).then_some(Chunk { start_ms, samples })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: usize = 16_000;

    fn tone(seconds: f32) -> Vec<f32> {
        (0..(seconds * SR as f32) as usize)
            .map(|i| 0.2 * (i as f32 * 440.0 * std::f32::consts::TAU / SR as f32).sin())
            .collect()
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * SR as f32) as usize]
    }

    fn secs(chunk: &Chunk) -> f32 {
        chunk.samples.len() as f32 / SR as f32
    }

    #[test]
    fn cuts_at_silence_after_minimum() {
        let mut chunker = Chunker::new(SR as u32);
        let audio = [tone(20.0), silence(1.0), tone(5.0)].concat();
        let chunks = chunker.push(&audio);
        assert_eq!(chunks.len(), 1);
        assert!(
            (20.0..=21.0).contains(&secs(&chunks[0])),
            "{}",
            secs(&chunks[0])
        );
    }

    #[test]
    fn hard_cut_at_maximum() {
        let mut chunker = Chunker::new(SR as u32);
        let chunks = chunker.push(&tone(40.0));
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].samples.len(), 30 * SR);
    }

    #[test]
    fn start_times_are_cumulative() {
        let mut chunker = Chunker::new(SR as u32);
        let mut chunks = chunker.push(&tone(65.0));
        chunks.extend(chunker.flush());
        assert_eq!(chunks[0].start_ms, 0);
        assert_eq!(chunks[1].start_ms, 30_000);
        assert_eq!(chunks[2].start_ms, 60_000);
    }

    #[test]
    fn silent_chunks_are_skipped_but_time_advances() {
        let mut chunker = Chunker::new(SR as u32);
        let mut chunks = chunker.push(&[silence(30.0), tone(20.0)].concat());
        chunks.extend(chunker.flush());
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].start_ms >= 30_000, "{}", chunks[0].start_ms);
    }

    #[test]
    fn flush_returns_remainder() {
        let mut chunker = Chunker::new(SR as u32);
        assert!(chunker.push(&tone(5.0)).is_empty());
        let rest = chunker.flush().unwrap();
        assert_eq!(rest.samples.len(), 5 * SR);
        assert!(chunker.flush().is_none());
    }

    #[test]
    fn push_in_small_buffers_equals_one_big_push() {
        let audio = [tone(20.0), silence(1.0), tone(15.0)].concat();
        let mut big = Chunker::new(SR as u32);
        let mut expected = big.push(&audio);
        expected.extend(big.flush());
        let mut small = Chunker::new(SR as u32);
        let mut actual: Vec<Chunk> = audio.chunks(1600).flat_map(|b| small.push(b)).collect();
        actual.extend(small.flush());
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(&expected) {
            assert_eq!((a.start_ms, a.samples.len()), (e.start_ms, e.samples.len()));
        }
    }

    #[test]
    fn interrupted_track_preserves_session_clock_without_materializing_silence() {
        let mut chunker = Chunker::new(SR as u32);
        chunker.push(&tone(3.0));
        assert_eq!(chunker.skip_to(3_600 * SR as u64).unwrap().start_ms, 0);
        chunker.push(&tone(2.0));
        let resumed = chunker.flush().unwrap();
        assert_eq!(resumed.start_ms, 3_600_000);
        assert_eq!(resumed.samples.len(), 2 * SR);
    }
}
