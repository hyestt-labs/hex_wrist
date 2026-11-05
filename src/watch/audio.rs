//! The watch's recording as samples: the 16 kHz mono AAC `.m4a` it sends,
//! decoded with symphonia into the 16 kHz f32 mono clip the dictation
//! engine takes. Other sample rates go through Hex's resampler.

use std::fmt;
use std::io::{Cursor, ErrorKind};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_AAC, CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub(crate) const SAMPLE_RATE: u32 = 16_000;

/// Why a recording could not be decoded; `Display` gives the text the watch
/// reads in `{"state":"failed","error":...}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecodeError {
    Unreadable(String),
    NoAudio,
    Unsupported(String),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "The recording could not be read: {why}"),
            Self::NoAudio => f.write_str("The recording has no audio"),
            Self::Unsupported(codec) => write!(f, "The recording format is not supported: {codec}"),
        }
    }
}

impl std::error::Error for DecodeError {}

fn unreadable(error: impl fmt::Display) -> DecodeError {
    DecodeError::Unreadable(error.to_string())
}

/// Decodes an `.m4a` (MP4 container, AAC) into 16 kHz mono f32 samples.
/// Any channel count is averaged to mono; any other sample rate goes through
/// `crate::dictation::try_resample_for_parakeet(&samples, source_rate)`.
pub(crate) fn decode_m4a(bytes: &[u8]) -> Result<Vec<f32>, DecodeError> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a").mime_type("audio/mp4");
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(unreadable)?
        .format;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(DecodeError::NoAudio)?;
    if track.codec_params.codec != CODEC_TYPE_AAC {
        return Err(DecodeError::Unsupported(format!(
            "codec {}",
            track.codec_params.codec
        )));
    }
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(unreadable)?;

    let mut mono: Vec<f32> = Vec::new();
    let mut source_rate = track.codec_params.sample_rate;
    let mut scratch: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(error) if is_end_of_stream(&error) => break,
            Err(error) => return Err(unreadable(error)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(error) if is_end_of_stream(&error) => break,
            // A corrupt packet loses its frame; later packets may still decode.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(unreadable(error)),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        if channels == 0 || decoded.frames() == 0 {
            continue;
        }
        source_rate = Some(spec.rate);
        let buffer = scratch
            .get_or_insert_with(|| SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        if buffer.capacity() < decoded.frames() * channels {
            *buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        }
        buffer.copy_interleaved_ref(decoded);
        mono.extend(
            buffer
                .samples()
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
    }

    if mono.is_empty() {
        return Err(DecodeError::NoAudio);
    }
    let rate = source_rate.filter(|rate| *rate > 0).ok_or_else(|| {
        DecodeError::Unreadable("the recording does not state a sample rate".to_string())
    })?;
    if rate == SAMPLE_RATE {
        return Ok(mono);
    }
    crate::dictation::try_resample_for_parakeet(&mono, rate).map_err(DecodeError::Unreadable)
}

fn is_end_of_stream(error: &SymphoniaError) -> bool {
    match error {
        SymphoniaError::IoError(io) => io.kind() == ErrorKind::UnexpectedEof,
        SymphoniaError::ResetRequired => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    // Fixtures: a 1.0 s, 440 Hz, amplitude 0.5 sine (16-bit mono WAV written by
    // a short Python `wave` script at the named rate), then encoded with:
    //   afconvert -f m4af -d aac -b 32000 tone-440-16k.wav tone-440-16k.m4a
    //   afconvert -f m4af -d aac@44100 -b 32000 tone-440-44k1.wav tone-440-44k1.m4a
    // (`aac@44100` keeps the source rate; without it afconvert picks 32 kHz.)
    use super::*;
    use std::panic::catch_unwind;

    const TONE_16K: &[u8] = include_bytes!("../../tests/fixtures/watch-audio/tone-440-16k.m4a");
    const TONE_44K1: &[u8] = include_bytes!("../../tests/fixtures/watch-audio/tone-440-44k1.m4a");

    fn goertzel_power(samples: &[f32], frequency: f32) -> f32 {
        let omega = 2.0 * std::f32::consts::PI * frequency / SAMPLE_RATE as f32;
        let coeff = 2.0 * omega.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &sample in samples {
            let s0 = sample + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        s1 * s1 + s2 * s2 - coeff * s1 * s2
    }

    fn assert_tone(samples: &[f32]) {
        assert!(
            (15_000..=19_000).contains(&samples.len()),
            "got {} samples",
            samples.len()
        );
        let peak = samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        assert!((0.3..=0.7).contains(&peak), "peak {peak}");
        let at_tone = goertzel_power(samples, 440.0);
        let off_tone = goertzel_power(samples, 1000.0);
        assert!(
            at_tone > 10.0 * off_tone,
            "440 Hz {at_tone}, 1 kHz {off_tone}"
        );
    }

    #[test]
    fn decodes_a_16k_tone() {
        assert_tone(&decode_m4a(TONE_16K).expect("decode"));
    }

    #[test]
    fn resamples_a_44k1_tone_to_16k() {
        assert_tone(&decode_m4a(TONE_44K1).expect("decode"));
    }

    #[test]
    fn hostile_input_errors_without_panicking() {
        let garbage: Vec<u8> = (0..64u32).map(|i| (i * 37 + 11) as u8).collect();
        let mut zeroed = TONE_16K.to_vec();
        let (start, end) = (zeroed.len() / 4, zeroed.len() * 3 / 4);
        zeroed[start..end].fill(0);

        let must_fail: [(&str, Vec<u8>); 3] = [
            ("empty", Vec::new()),
            ("garbage", garbage),
            ("truncated to 64 bytes", TONE_16K[..64].to_vec()),
        ];
        for (name, input) in must_fail {
            let result =
                catch_unwind(|| decode_m4a(&input)).unwrap_or_else(|_| panic!("{name} panicked"));
            assert!(result.is_err(), "{name} decoded");
        }

        let result = catch_unwind(|| decode_m4a(&zeroed)).expect("zeroed middle panicked");
        if let Ok(samples) = result {
            assert!(!samples.is_empty());
        }
    }

    #[test]
    fn a_wav_file_is_rejected() {
        let data = [0u8; 320];
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&32_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        assert!(decode_m4a(&wav).is_err());
    }

    #[test]
    fn errors_read_as_text_for_the_watch() {
        assert_eq!(
            DecodeError::NoAudio.to_string(),
            "The recording has no audio"
        );
    }
}
