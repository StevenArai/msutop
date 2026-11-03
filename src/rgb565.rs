//! The RAW FULL + pipeline path used by ../RAW_DIRECT_POC.py.
//! R-mode headers are little-endian; RGB565 pixel payloads are big-endian.
use crate::{frame::FrameSink, Frame, HEIGHT, WIDTH};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

const MAGIC: &[u8; 4] = b"RG56";
const HANDSHAKE: &[u8; 6] = b"\0MSNCR";
const HEADER_BYTES: usize = 25;
const ACK_BYTES: usize = 12;
const BAND_ROWS: usize = 8;
const BAND_BYTES: usize = WIDTH * BAND_ROWS * 2;
const BANDS: usize = HEIGHT / BAND_ROWS;
pub const WIRE_BYTES: usize = BANDS * (HEADER_BYTES + BAND_BYTES) + HEADER_BYTES;

const CRC_TABLE: [u32; 256] = {
    let mut table = [0; 256];
    let mut index = 0;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = (value >> 1) ^ if value & 1 != 0 { 0xedb8_8320 } else { 0 };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
};

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc = (crc >> 8) ^ CRC_TABLE[((crc as u8) ^ byte) as usize];
    }
    !crc
}

pub struct Encoder {
    wire: Vec<u8>,
}

impl Default for Encoder {
    fn default() -> Self {
        Self {
            wire: vec![0; WIRE_BYTES],
        }
    }
}

impl Encoder {
    /// Ten RAW bands and COMMIT, with complete coverage on every frame.
    pub fn encode(&mut self, frame: &Frame, session: u16, number: u16) -> &[u8] {
        for band in 0..BANDS {
            let offset = band * (HEADER_BYTES + BAND_BYTES);
            let (header, payload) =
                self.wire[offset..offset + HEADER_BYTES + BAND_BYTES].split_at_mut(HEADER_BYTES);
            let first = band * WIDTH * BAND_ROWS;
            for (pixel, bytes) in frame.pixels[first..first + WIDTH * BAND_ROWS]
                .iter()
                .zip(payload.as_chunks_mut::<2>().0)
            {
                *bytes = pixel.to_565().to_be_bytes();
            }
            let crc = crc32(payload);
            fill_header(header, session, number, band as u16, BAND_BYTES as u16, crc);
        }
        fill_header(
            &mut self.wire[BANDS * (HEADER_BYTES + BAND_BYTES)..],
            session,
            number,
            BANDS as u16,
            0,
            0,
        );
        &self.wire
    }
}

fn fill_header(header: &mut [u8], session: u16, frame: u16, band: u16, bytes: u16, crc: u32) {
    header.fill(0);
    header[..4].copy_from_slice(MAGIC);
    header[4] = 1;
    header[5] = if bytes == 0 { 2 } else { 1 };
    header[6..8].copy_from_slice(&session.to_le_bytes());
    header[8..10].copy_from_slice(&frame.to_le_bytes());
    header[10..12].copy_from_slice(&band.to_le_bytes());
    header[12..14].copy_from_slice(&bytes.to_le_bytes());
    // Encoding 0 = RAW. Compressed and plain lengths/CRCs are identical.
    header[15..17].copy_from_slice(&bytes.to_le_bytes());
    header[17..21].copy_from_slice(&crc.to_le_bytes());
    header[21..25].copy_from_slice(&crc.to_le_bytes());
}

/// Queue inspection detects stale or extra replies at frame boundaries.
pub trait Port: Read + Write {
    fn pending_bytes(&self) -> io::Result<usize>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub frames: u64,
    pub wire_bytes: u64,
    pub reply_bytes: u64,
}

/// One whole frame in flight. All eleven numbered ACKs are checked before reuse.
pub struct Rgb565Sink<P> {
    port: P,
    encoder: Encoder,
    timeout: Duration,
    failed: bool,
    session: u16,
    number: u16,
    stats: Stats,
}

impl<P: Port> Rgb565Sink<P> {
    /// Only use when the device's R mode has already been confirmed.
    pub fn resume(port: P, session: u16, timeout: Duration) -> Self {
        Self {
            port,
            encoder: Encoder::default(),
            timeout,
            failed: false,
            session,
            number: 0,
            stats: Stats::default(),
        }
    }

    pub fn connect(port: P, session: u16, timeout: Duration) -> io::Result<Self> {
        let mut sink = Self::resume(port, session, timeout);
        let deadline = Instant::now() + timeout;
        sink.write_bytes(HANDSHAKE, deadline)?;
        let mut token = [0; 6];
        let mut received = 0;
        loop {
            let mut byte = [0];
            sink.read_bytes(&mut byte, deadline)?;
            token.rotate_left(1);
            token[5] = byte[0];
            received += 1;
            if &token == HANDSHAKE {
                return Ok(sink);
            }
            if received >= 4096 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "RGB565 R handshake reply limit exceeded",
                ));
            }
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    fn write_bytes(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<()> {
        let mut offset = 0;
        while offset < bytes.len() {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "RGB565 write deadline exceeded",
                ));
            }
            match self.port.write(&bytes[offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "RGB565 port accepted no bytes",
                    ))
                }
                Ok(count) => {
                    offset += count;
                    self.stats.wire_bytes += count as u64;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn read_bytes(&mut self, bytes: &mut [u8], deadline: Instant) -> io::Result<usize> {
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "RGB565 R reply timeout; reset to terminal mode before a new handshake",
                ));
            }
            match self.port.read(bytes) {
                Ok(0) => std::thread::sleep(Duration::from_millis(1)),
                Ok(count) => {
                    self.stats.reply_bytes += count as u64;
                    return Ok(count);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn check_empty(&self) -> io::Result<()> {
        if self.port.pending_bytes()? != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "RGB565 unconsumed replies at frame boundary",
            ));
        }
        Ok(())
    }

    fn transmit(&mut self, frame: &Frame) -> io::Result<()> {
        self.check_empty()?;
        self.encoder.encode(frame, self.session, self.number);
        let deadline = Instant::now() + self.timeout;
        // Keep the buffer across frames without copying 25 KB for borrow splitting.
        let mut wire = std::mem::take(&mut self.encoder.wire);
        let result = self.write_bytes(&wire, deadline);
        std::mem::swap(&mut self.encoder.wire, &mut wire);
        result?;
        let mut replies = [0; ACK_BYTES * (BANDS + 1)];
        let mut received = 0;
        let mut checked = 0;
        while received < replies.len() {
            received += self.read_bytes(&mut replies[received..], deadline)?;
            while checked < BANDS + 1 && received >= (checked + 1) * ACK_BYTES {
                let reply = &replies[checked * ACK_BYTES..(checked + 1) * ACK_BYTES];
                let session = u16::from_le_bytes([reply[5], reply[6]]);
                let number = u16::from_le_bytes([reply[7], reply[8]]);
                let band = u16::from_le_bytes([reply[9], reply[10]]);
                if reply[..4] != *MAGIC
                    || reply[4] != 0x80
                    || reply[11] != 0
                    || (session, number, band) != (self.session, self.number, checked as u16)
                {
                    return Err(io::Error::new(io::ErrorKind::InvalidData,
                        format!("RGB565 rejected or mismatched ACK at session {}, frame {}, band {}: {reply:02x?}", self.session, self.number, checked)));
                }
                checked += 1;
            }
        }
        self.check_empty()?;
        self.stats.frames += 1;
        self.number = self.number.wrapping_add(1);
        if self.number == 0 {
            self.session = self.session.wrapping_add(1);
        }
        Ok(())
    }
}

impl<P: Port> FrameSink for Rgb565Sink<P> {
    fn present(&mut self, frame: &Frame) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "RGB565 session has already failed",
            ));
        }
        let result = self.transmit(frame);
        self.failed = result.is_err();
        result
    }
}

#[cfg(feature = "serial")]
impl Port for Box<dyn serialport::SerialPort> {
    fn pending_bytes(&self) -> io::Result<usize> {
        self.bytes_to_read()
            .map(|count| count as usize)
            .map_err(io::Error::other)
    }
}

#[cfg(feature = "serial")]
pub fn open(
    port: &str,
    baud: u32,
    already_in_mode: bool,
) -> io::Result<Rgb565Sink<Box<dyn serialport::SerialPort>>> {
    let port = serialport::new(port, baud)
        .timeout(Duration::from_millis(10))
        .open()
        .map_err(io::Error::other)?;
    let session = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |time| (time.as_micros() as u16).max(1));
    let timeout = Duration::from_secs(5);
    if already_in_mode {
        Ok(Rgb565Sink::resume(port, session, timeout))
    } else {
        Rgb565Sink::connect(port, session, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rgb;
    use std::collections::VecDeque;

    #[test]
    fn raw_full_preserves_every_pixel_and_matches_poc_header_vectors() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        let mut frame = Frame::default();
        for (index, pixel) in frame.pixels.iter_mut().enumerate() {
            *pixel = Rgb::from_565((index * 5) as u16);
        }
        let mut encoder = Encoder::default();
        let wire = encoder.encode(&frame, 0x1234, 0x5678);
        assert_eq!(wire.len(), 25875);
        for band in 0..BANDS {
            let offset = band * (HEADER_BYTES + BAND_BYTES);
            let header = &wire[offset..offset + HEADER_BYTES];
            assert_eq!(
                &header[..10],
                &[0x52, 0x47, 0x35, 0x36, 1, 1, 0x34, 0x12, 0x78, 0x56]
            );
            assert_eq!(&header[10..17], &[band as u8, 0, 0, 10, 0, 0, 10]);
            let pixels = wire[offset + HEADER_BYTES..offset + HEADER_BYTES + BAND_BYTES]
                .as_chunks::<2>()
                .0;
            for (index, &bytes) in pixels.iter().enumerate() {
                assert_eq!(
                    u16::from_be_bytes(bytes),
                    ((band * 1280 + index) * 5) as u16
                );
            }
            let expected = crc32(&wire[offset + HEADER_BYTES..offset + HEADER_BYTES + BAND_BYTES])
                .to_le_bytes();
            assert_eq!(&header[17..21], &expected);
            assert_eq!(&header[21..25], &expected);
        }
        assert_eq!(
            &wire[WIRE_BYTES - HEADER_BYTES..],
            &[
                0x52, 0x47, 0x35, 0x36, 1, 2, 0x34, 0x12, 0x78, 0x56, 10, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0,
            ]
        );
    }

    #[derive(Clone, Copy)]
    enum Fault {
        None,
        Stale,
        Duplicate,
        Reject,
        Missing,
        Extra,
    }

    struct Peer {
        incoming: Vec<u8>,
        outgoing: VecDeque<u8>,
        handshaken: bool,
        short_read: usize,
        commits: usize,
        read_before_commit: bool,
        fault: Fault,
        expected: Vec<u8>,
    }

    impl Peer {
        fn new(short_read: usize, fault: Fault) -> Self {
            Self {
                incoming: Vec::new(),
                outgoing: VecDeque::new(),
                handshaken: false,
                short_read,
                commits: 0,
                read_before_commit: false,
                fault,
                expected: vec![0; WIDTH * HEIGHT * 2],
            }
        }
    }

    impl Read for Peer {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.handshaken && self.commits == 0 && self.outgoing.len() > 6 {
                self.read_before_commit = true;
            }
            let count = output.len().min(self.short_read).min(self.outgoing.len());
            for byte in &mut output[..count] {
                *byte = self.outgoing.pop_front().unwrap();
            }
            if count == 0 {
                Err(io::ErrorKind::WouldBlock.into())
            } else {
                Ok(count)
            }
        }
    }

    impl Write for Peer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(19);
            self.incoming.extend_from_slice(&bytes[..count]);
            if !self.handshaken {
                if self.incoming.len() >= 6 {
                    assert_eq!(&self.incoming[..6], HANDSHAKE);
                    self.incoming.drain(..6);
                    self.outgoing.extend(HANDSHAKE);
                    self.handshaken = true;
                }
                return Ok(count);
            }
            while self.incoming.len() >= HEADER_BYTES {
                let length = u16::from_le_bytes([self.incoming[15], self.incoming[16]]) as usize;
                if self.incoming.len() < HEADER_BYTES + length {
                    break;
                }
                let packet: Vec<_> = self.incoming.drain(..HEADER_BYTES + length).collect();
                assert_eq!(&packet[..4], MAGIC);
                assert_eq!(packet[4], 1);
                assert_eq!(packet[14], 0);
                let band = u16::from_le_bytes([packet[10], packet[11]]) as usize;
                if band < BANDS {
                    assert_eq!(packet[5], 1);
                    assert_eq!(length, BAND_BYTES);
                    assert_eq!(
                        &packet[HEADER_BYTES..],
                        &self.expected[band * BAND_BYTES..(band + 1) * BAND_BYTES]
                    );
                    assert_eq!(
                        &packet[17..21],
                        &crc32(&packet[HEADER_BYTES..]).to_le_bytes()
                    );
                    assert_eq!(&packet[17..21], &packet[21..25]);
                } else {
                    assert_eq!(band, BANDS);
                    assert_eq!(packet[5], 2);
                    assert_eq!(&packet[12..], &[0; 13]);
                    self.commits += 1;
                }
                let mut reply = [0; ACK_BYTES];
                reply[..4].copy_from_slice(MAGIC);
                reply[4] = 0x80;
                reply[5..11].copy_from_slice(&packet[6..12]);
                if band == 3 {
                    match self.fault {
                        Fault::Stale => reply[7] ^= 1,
                        Fault::Reject => {
                            reply[4] = 0x81;
                            reply[11] = 4;
                        }
                        Fault::Duplicate => {
                            self.outgoing.extend(reply);
                        }
                        Fault::Missing => continue,
                        _ => (),
                    }
                }
                self.outgoing.extend(reply);
                if band == BANDS && matches!(self.fault, Fault::Extra) {
                    self.outgoing.push_back(0xff);
                }
            }
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Port for Peer {
        fn pending_bytes(&self) -> io::Result<usize> {
            Ok(self.outgoing.len())
        }
    }

    #[test]
    fn pipeline_handles_split_acks_short_writes_and_frame_id_wrap() {
        for read_size in [1, 7, 64, 132] {
            let peer = Peer::new(read_size, Fault::None);
            let mut sink = Rgb565Sink::connect(peer, 7, Duration::from_secs(1)).unwrap();
            sink.number = u16::MAX;
            sink.present(&Frame::default()).unwrap();
            assert_eq!((sink.session, sink.number), (8, 0));
            sink.present(&Frame::default()).unwrap();
            assert_eq!(sink.port.commits, 2);
            assert!(
                !sink.port.read_before_commit,
                "pipeline must write the whole first frame before reading ACKs"
            );
            assert_eq!(sink.stats.frames, 2);
            assert_eq!(sink.stats.wire_bytes, 6 + 2 * WIRE_BYTES as u64);
            assert_eq!(sink.stats.reply_bytes, 6 + 2 * 132);
            assert_eq!(sink.port.pending_bytes().unwrap(), 0);
        }
    }

    #[test]
    fn stale_duplicate_nak_missing_or_extra_acks_stop_the_session() {
        for fault in [
            Fault::Stale,
            Fault::Duplicate,
            Fault::Reject,
            Fault::Missing,
            Fault::Extra,
        ] {
            let peer = Peer::new(7, fault);
            let mut sink = Rgb565Sink::connect(peer, 7, Duration::from_millis(20)).unwrap();
            assert!(sink.present(&Frame::default()).is_err());
            assert_eq!(sink.stats.frames, 0);
            let before = sink.stats.wire_bytes;
            assert!(sink.present(&Frame::default()).is_err());
            assert_eq!(sink.stats.wire_bytes, before);
        }
    }
}
