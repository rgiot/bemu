//! The sound, exported: the AY's registers frame by frame as a YM file (for a tracker or a
//! player, or to compare a music player with the tune), the sound itself as a WAV file.

use std::io::{Seek, SeekFrom, Write};

/// A YM6 file, uncompressed (the players read it so; LHA packing is optional): the 16
/// registers of each frame, interleaved register by register.
pub fn ym6(frames: &[[u8; 16]], frame_rate: u16, song: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(64 + frames.len() * 16);
    bytes.extend_from_slice(b"YM6!LeOnArD!");
    bytes.extend_from_slice(&(frames.len() as u32).to_be_bytes());
    // attributes: bit 0, the registers are interleaved
    bytes.extend_from_slice(&1u32.to_be_bytes());
    // no digidrums
    bytes.extend_from_slice(&0u16.to_be_bytes());
    // the CPC's AY runs at 1 MHz
    bytes.extend_from_slice(&1_000_000u32.to_be_bytes());
    bytes.extend_from_slice(&frame_rate.to_be_bytes());
    // loop frame, size of the additional data
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes());
    for text in [song, "", "bemu"] {
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(0);
    }
    for register in 0..16 {
        bytes.extend(frames.iter().map(|frame| frame[register]));
    }
    bytes.extend_from_slice(b"End!");
    bytes
}

/// A 16-bit PCM WAV file, written as the samples come; the header's sizes are set by
/// [`Self::finish`].
pub struct WavWriter<W: Write + Seek> {
    out: W,
    channels: u16,
    data_bytes: u32
}

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut out: W, sample_rate: u32, channels: u16) -> std::io::Result<Self> {
        let block_align = channels * 2;
        out.write_all(b"RIFF")?;
        out.write_all(&0u32.to_le_bytes())?;
        out.write_all(b"WAVEfmt ")?;
        out.write_all(&16u32.to_le_bytes())?;
        out.write_all(&1u16.to_le_bytes())?; // PCM
        out.write_all(&channels.to_le_bytes())?;
        out.write_all(&sample_rate.to_le_bytes())?;
        out.write_all(&(sample_rate * u32::from(block_align)).to_le_bytes())?;
        out.write_all(&block_align.to_le_bytes())?;
        out.write_all(&16u16.to_le_bytes())?;
        out.write_all(b"data")?;
        out.write_all(&0u32.to_le_bytes())?;
        Ok(Self { out, channels, data_bytes: 0 })
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Samples in -1..1, interleaved as the file's channels.
    pub fn samples(&mut self, samples: &[f32]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            bytes.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        self.out.write_all(&bytes)?;
        self.data_bytes = self.data_bytes.saturating_add(bytes.len() as u32);
        Ok(())
    }

    /// Set the sizes in the header.
    pub fn finish(mut self) -> std::io::Result<W> {
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(36 + self.data_bytes).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(40))?;
        self.out.write_all(&self.data_bytes.to_le_bytes())?;
        self.out.flush()?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_ym6_file_interleaves_the_registers() {
        let mut first = [0u8; 16];
        first[0] = 0x12;
        first[13] = 0xff;
        let mut second = [0u8; 16];
        second[0] = 0x34;
        second[13] = 0x0e;
        let ym = ym6(&[first, second], 50, "tune");
        assert_eq!(&ym[..12], b"YM6!LeOnArD!");
        assert_eq!(u32::from_be_bytes(ym[12..16].try_into().unwrap()), 2);
        assert_eq!(u32::from_be_bytes(ym[22..26].try_into().unwrap()), 1_000_000);
        assert_eq!(u16::from_be_bytes(ym[26..28].try_into().unwrap()), 50);
        let data = ym.len() - 4 - 32;
        assert_eq!(&ym[data..data + 2], &[0x12, 0x34], "register 0 of both frames");
        assert_eq!(&ym[data + 26..data + 28], &[0xff, 0x0e], "register 13");
        assert!(ym.ends_with(b"End!"));
    }

    #[test]
    fn a_wav_file_has_its_sizes() {
        let mut wav = WavWriter::new(std::io::Cursor::new(Vec::new()), 44_100, 2).unwrap();
        wav.samples(&[0.0, 0.5, -1.0, 1.0]).unwrap();
        let bytes = wav.finish().unwrap().into_inner();
        assert_eq!(bytes.len(), 44 + 8);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 8);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 16383);
        assert_eq!(i16::from_le_bytes([bytes[48], bytes[49]]), -32767);
    }
}
