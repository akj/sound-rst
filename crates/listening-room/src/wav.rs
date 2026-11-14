//! Writes a take as a WAV file: two channels of 32-bit float samples.

use std::io::{self, Write};
use std::path::Path;

use crate::scene::RATE;

/// Writes interleaved left and right samples at [`RATE`].
pub fn write(path: &Path, samples: &[f32]) -> io::Result<()> {
    let data_len = u32::try_from(samples.len() * 4)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "take too long for WAV"))?;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 4);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    bytes.extend_from_slice(&2u16.to_le_bytes()); // channels
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 8).to_le_bytes()); // bytes a second
    bytes.extend_from_slice(&8u16.to_le_bytes()); // bytes a frame
    bytes.extend_from_slice(&32u16.to_le_bytes()); // bits a sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::File::create(path)?.write_all(&bytes)
}
