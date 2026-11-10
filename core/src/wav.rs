/// Exclusive millisecond ends must cover the final sample, including a fractional millisecond.
pub(crate) fn sample_duration_ms(sample_count: u64, sample_rate: u32) -> u64 {
    sample_count
        .saturating_mul(1_000)
        .div_ceil(u64::from(sample_rate.max(1)))
}

pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// Decodes 16-bit mono PCM WAV, skipping chunks other than `fmt ` and `data`.
pub fn decode_wav(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let mut sample_rate = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = bytes.get(pos + 8..pos + 8 + len)?;
        match id {
            b"fmt " => {
                let channels = u16::from_le_bytes(body.get(2..4)?.try_into().ok()?);
                let bits = u16::from_le_bytes(body.get(14..16)?.try_into().ok()?);
                if channels != 1 || bits != 16 {
                    return None;
                }
                sample_rate = Some(u32::from_le_bytes(body.get(4..8)?.try_into().ok()?));
            }
            b"data" => {
                let samples = body
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| f32::from(i16::from_le_bytes(*b)) / 32767.0)
                    .collect();
                return sample_rate.map(|rate| (samples, rate));
            }
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_sample_durations_cover_submilliseconds_without_changing_integral_ends() {
        for (samples, expected) in [
            (0, 0),
            (1, 1),
            (15, 1),
            (16, 1),
            (17, 2),
            (122_229, 7_640),
            (272_326, 17_021),
        ] {
            assert_eq!(sample_duration_ms(samples, 16_000), expected);
        }
    }

    #[test]
    fn wav_header_is_valid() {
        let wav = encode_wav(&vec![0.0; 16000], 16000);
        assert_eq!(wav.len(), 44 + 32000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16000);
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
    }

    #[test]
    fn decode_roundtrips_encode() {
        let samples = [0.0, 0.5, -0.5, 0.25];
        let (decoded, rate) = decode_wav(&encode_wav(&samples, 16000)).unwrap();
        assert_eq!(rate, 16000);
        for (a, b) in samples.iter().zip(&decoded) {
            assert!((a - b).abs() < 0.001, "{a} vs {b}");
        }
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_wav(b"not a wav").is_none());
    }

    #[test]
    fn wav_clamps_out_of_range() {
        let wav = encode_wav(&[2.0, -2.0], 16000);
        assert_eq!(i16::from_le_bytes(wav[44..46].try_into().unwrap()), 32767);
        assert_eq!(i16::from_le_bytes(wav[46..48].try_into().unwrap()), -32767);
    }
}
