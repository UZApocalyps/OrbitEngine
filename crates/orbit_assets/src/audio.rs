use std::io::Cursor;

/// Decoded audio: interleaved `f32` samples in `-1.0..=1.0`.
#[derive(Debug, Clone)]
pub struct PcmAudio {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl PcmAudio {
    /// Number of frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration_secs(&self) -> f32 {
        self.frames() as f32 / self.sample_rate.max(1) as f32
    }
}

/// Decodes WAV or OGG Vorbis. The format is picked from the magic bytes,
/// falling back to the file extension of `name`.
pub fn decode_audio(name: &str, bytes: &[u8]) -> Result<PcmAudio, String> {
    let lower = name.to_ascii_lowercase();
    if bytes.starts_with(b"RIFF") || (!bytes.starts_with(b"OggS") && lower.ends_with(".wav")) {
        decode_wav(bytes)
    } else if bytes.starts_with(b"OggS") || lower.ends_with(".ogg") {
        decode_ogg(bytes)
    } else {
        Err("unsupported audio format (expected .ogg or .wav)".to_owned())
    }
}

fn decode_wav(bytes: &[u8]) -> Result<PcmAudio, String> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.channels == 0 {
        return Err("wav has zero channels".to_owned());
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?,
        hound::SampleFormat::Int => {
            if spec.bits_per_sample == 0 || spec.bits_per_sample > 32 {
                return Err(format!("unsupported bit depth {}", spec.bits_per_sample));
            }
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        }
    };
    Ok(PcmAudio {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples,
    })
}

fn decode_ogg(bytes: &[u8]) -> Result<PcmAudio, String> {
    let mut reader =
        lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let channels = reader.ident_hdr.audio_channels as u16;
    if channels == 0 {
        return Err("ogg has zero channels".to_owned());
    }
    let mut samples = Vec::new();
    while let Some(packet) = reader.read_dec_packet_itl().map_err(|e| e.to_string())? {
        samples.extend(packet.into_iter().map(|s| s as f32 / 32768.0));
    }
    Ok(PcmAudio {
        sample_rate,
        channels,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_bytes(spec: hound::WavSpec, samples: &[f32], int16: bool) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for &s in samples {
                if int16 {
                    w.write_sample((s * 32767.0) as i16).unwrap();
                } else {
                    w.write_sample(s).unwrap();
                }
            }
            w.finalize().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn decodes_16bit_mono_wav() {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 22_050,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let bytes = wav_bytes(spec, &[0.0, 0.5, -1.0], true);
        let pcm = decode_audio("x.wav", &bytes).unwrap();
        assert_eq!(
            (pcm.sample_rate, pcm.channels, pcm.frames()),
            (22_050, 1, 3)
        );
        assert!((pcm.samples[1] - 0.5).abs() < 1e-3);
        assert!((pcm.samples[2] + 1.0).abs() < 1e-3);
    }

    #[test]
    fn decodes_float_stereo_wav() {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let bytes = wav_bytes(spec, &[0.25, -0.25, 0.5, -0.5], false);
        let pcm = decode_audio("x.wav", &bytes).unwrap();
        assert_eq!((pcm.channels, pcm.frames()), (2, 2));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_audio("a.ogg", b"OggS garbage").is_err());
        assert!(decode_audio("a.wav", b"RIFF garbage").is_err());
        assert!(decode_audio("a.mp3", b"ID3").is_err());
    }
}
