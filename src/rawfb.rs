//! Legacy MSU image commands. Pixel pairs contain big-endian RGB565 values.
//! The repository's N-mode firmware quantizes these values to its 16-color palette.
use crate::{frame::FrameSink, Frame, HEIGHT, WIDTH};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

const COMMAND_BYTES: usize = 6;
const SEGMENT_PIXELS: usize = 128;
const PAIRS: usize = SEGMENT_PIXELS / 2;
const SEGMENTS: usize = WIDTH * HEIGHT / SEGMENT_PIXELS;
const HANDSHAKE: &[u8; 6] = b"\0MSNCN";
const ACK: [u8; 2] = [0x02, 0x03];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AckMode {
    /// Repository firmware acknowledges every six-byte command.
    #[default]
    Commands,
    /// Original host waits only for BEGIN and each segment COMMIT.
    Segments,
}

impl AckMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "commands" => Some(Self::Commands),
            "segments" => Some(Self::Segments),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Segments => "segments",
        }
    }
}

pub struct Encoder {
    wire: Vec<u8>,
    ends: [usize; SEGMENTS],
}

impl Default for Encoder {
    fn default() -> Self {
        Self {
            wire: Vec::with_capacity(18 + SEGMENTS * (PAIRS + 2) * COMMAND_BYTES),
            ends: [0; SEGMENTS],
        }
    }
}

impl Encoder {
    /// Every frame explicitly resets the window and covers every pixel.
    /// A modal pair prefill saves commands without changing the RGB565 values.
    pub fn encode(&mut self, frame: &Frame) -> &[u8] {
        self.wire.clear();
        self.wire.extend_from_slice(&[0x02, 0x00, 0, 0, 0, 0]);
        self.wire
            .extend_from_slice(&[0x02, 0x01, 0, WIDTH as u8, 0, HEIGHT as u8]);
        self.wire.extend_from_slice(&[0x02, 0x03, 0x07, 0, 0, 0]);
        for (segment, pixels) in frame
            .pixels
            .as_chunks::<SEGMENT_PIXELS>()
            .0
            .iter()
            .enumerate()
        {
            let pairs: [u32; PAIRS] = std::array::from_fn(|index| {
                ((pixels[index * 2].to_565() as u32) << 16) | pixels[index * 2 + 1].to_565() as u32
            });
            let mut sorted = pairs;
            sorted.sort_unstable();
            let mut main = sorted[0];
            let mut best = 0;
            let mut start = 0;
            while start < PAIRS {
                let mut end = start + 1;
                while end < PAIRS && sorted[end] == sorted[start] {
                    end += 1;
                }
                if end - start > best {
                    main = sorted[start];
                    best = end - start;
                }
                start = end;
            }
            self.wire.extend_from_slice(&[0x02, 0x04]);
            self.wire.extend_from_slice(&main.to_be_bytes());
            for (index, &pair) in pairs.iter().enumerate() {
                if pair != main {
                    self.wire.extend_from_slice(&[0x04, index as u8]);
                    self.wire.extend_from_slice(&pair.to_be_bytes());
                }
            }
            self.wire.extend_from_slice(&[0x02, 0x03, 0x08, 0x01, 0, 0]);
            self.ends[segment] = self.wire.len();
        }
        &self.wire
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub frames: u64,
    pub wire_bytes: u64,
    pub reply_bytes: u64,
}

/// A single in-flight frame, bounded segment batches and checked acknowledgments.
/// After any error the session stops; unnumbered legacy ACKs cannot safely recover.
pub struct RawfbSink<P> {
    port: P,
    encoder: Encoder,
    ack_mode: AckMode,
    timeout: Duration,
    failed: bool,
    stats: Stats,
}

impl<P: Read + Write> RawfbSink<P> {
    /// Resume a port whose N mode was already confirmed in the current session.
    /// The first frame still resets the window and checks the selected ACK contract.
    pub fn resume(port: P, ack_mode: AckMode, timeout: Duration) -> Self {
        Self {
            port,
            encoder: Encoder::default(),
            ack_mode,
            timeout,
            failed: false,
            stats: Stats::default(),
        }
    }

    /// The underlying port must use a short, finite read/write timeout.
    pub fn connect(port: P, ack_mode: AckMode, timeout: Duration) -> io::Result<Self> {
        let mut sink = Self::resume(port, ack_mode, timeout);
        let deadline = Instant::now() + timeout;
        sink.write_bytes(HANDSHAKE, deadline)?;
        let mut window = [0_u8; 6];
        let mut received = 0;
        loop {
            let byte = sink.read_byte(deadline)?;
            window.rotate_left(1);
            window[5] = byte;
            received += 1;
            if received >= 6 && &window == HANDSHAKE {
                return Ok(sink);
            }
            if received >= 4096 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "RAWFB handshake reply limit exceeded",
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
                    "RAWFB write deadline exceeded",
                ));
            }
            match self.port.write(&bytes[offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "RAWFB port accepted no bytes",
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

    fn read_byte(&mut self, deadline: Instant) -> io::Result<u8> {
        let mut byte = [0];
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "RAWFB reply timeout; reconnect in terminal mode before retrying",
                ));
            }
            match self.port.read(&mut byte) {
                Ok(0) => std::thread::sleep(Duration::from_millis(1)),
                Ok(_) => {
                    self.stats.reply_bytes += 1;
                    return Ok(byte[0]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
                Err(error) => return Err(error),
            }
        }
    }

    fn expect_acks(&mut self, count: usize, deadline: Instant) -> io::Result<()> {
        let mut bytes = [0; (PAIRS + 2) * 2];
        let length = count * 2;
        let mut offset = 0;
        while offset < length {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "RAWFB command ACK deadline exceeded",
                ));
            }
            match self.port.read(&mut bytes[offset..length]) {
                Ok(0) => std::thread::sleep(Duration::from_millis(1)),
                Ok(count) => {
                    offset += count;
                    self.stats.reply_bytes += count as u64;
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
        for reply in bytes[..length].as_chunks::<2>().0 {
            if *reply != ACK {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("RAWFB rejected a command or ACK mode differs: {reply:02x?}"),
                ));
            }
        }
        Ok(())
    }

    fn transmit(&mut self, frame: &Frame) -> io::Result<()> {
        let deadline = Instant::now() + self.timeout;
        self.encoder.encode(frame);
        let mut start = 0;
        // Copy one small batch out of the reusable encoder; never queue an entire frame.
        let mut batch = [0_u8; (PAIRS + 2) * COMMAND_BYTES];
        for end in std::iter::once(3 * COMMAND_BYTES).chain(self.encoder.ends) {
            let length = end - start;
            batch[..length].copy_from_slice(&self.encoder.wire[start..end]);
            self.write_bytes(&batch[..length], deadline)?;
            let replies = match self.ack_mode {
                AckMode::Commands => length / COMMAND_BYTES,
                AckMode::Segments => 1,
            };
            self.expect_acks(replies, deadline)?;
            start = end;
        }
        self.stats.frames += 1;
        Ok(())
    }
}

impl<P: Read + Write> FrameSink for RawfbSink<P> {
    fn present(&mut self, frame: &Frame) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "RAWFB session has already failed",
            ));
        }
        let result = self.transmit(frame);
        self.failed = result.is_err();
        result
    }
}

#[cfg(feature = "serial")]
pub fn open(
    port: &str,
    baud: u32,
    ack_mode: AckMode,
    already_in_mode: bool,
) -> io::Result<RawfbSink<Box<dyn serialport::SerialPort>>> {
    let port = serialport::new(port, baud)
        .timeout(Duration::from_millis(10))
        .open()
        .map_err(io::Error::other)?;
    let timeout = Duration::from_secs(5);
    if already_in_mode {
        Ok(RawfbSink::resume(port, ack_mode, timeout))
    } else {
        RawfbSink::connect(port, ack_mode, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rgb;
    use std::collections::VecDeque;

    #[test]
    fn full_frame_commands_preserve_pixel_order_and_restart_each_frame() {
        let mut encoder = Encoder::default();
        for solid in [true, false] {
            let mut frame = Frame::default();
            for (index, pixel) in frame.pixels.iter_mut().enumerate() {
                *pixel = if solid {
                    Rgb(0, 255, 255)
                } else {
                    Rgb((index * 31) as u8, (index * 17) as u8, (index * 7) as u8)
                };
            }
            let wire = encoder.encode(&frame);
            assert_eq!(
                &wire[..18],
                &[2, 0, 0, 0, 0, 0, 2, 1, 0, 160, 0, 80, 2, 3, 7, 0, 0, 0]
            );
            let mut decoded = vec![0_u16; WIDTH * HEIGHT];
            let mut segment = 0;
            let mut primed = false;
            for command in wire[18..].as_chunks::<6>().0 {
                if command[..2] == [2, 4] {
                    assert!(!primed);
                    let a = u16::from_be_bytes([command[2], command[3]]);
                    let b = u16::from_be_bytes([command[4], command[5]]);
                    for index in 0..SEGMENT_PIXELS {
                        decoded[segment * SEGMENT_PIXELS + index] =
                            if index % 2 == 0 { a } else { b };
                    }
                    primed = true;
                } else if command[0] == 4 {
                    assert!(primed && command[1] < 64);
                    let index = segment * SEGMENT_PIXELS + command[1] as usize * 2;
                    decoded[index] = u16::from_be_bytes([command[2], command[3]]);
                    decoded[index + 1] = u16::from_be_bytes([command[4], command[5]]);
                } else {
                    assert_eq!(command, &[2, 3, 8, 1, 0, 0]);
                    assert!(primed);
                    segment += 1;
                    primed = false;
                }
            }
            assert_eq!(segment, SEGMENTS);
            assert!(!primed);
            assert_eq!(
                decoded,
                frame.pixels.iter().map(|p| p.to_565()).collect::<Vec<_>>()
            );
            if solid {
                assert_eq!(wire.len(), 18 + SEGMENTS * 12);
            }
        }
    }

    struct Peer {
        pending: Vec<u8>,
        replies: VecDeque<u8>,
        ack_mode: AckMode,
        handshaken: bool,
        commits: usize,
        reject: bool,
    }
    impl Read for Peer {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if let Some(byte) = self.replies.pop_front() {
                output[0] = byte;
                Ok(1)
            } else {
                Err(io::ErrorKind::WouldBlock.into())
            }
        }
    }
    impl Write for Peer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(5);
            self.pending.extend_from_slice(&bytes[..count]);
            while self.pending.len() >= 6 {
                let command: Vec<_> = self.pending.drain(..6).collect();
                if !self.handshaken {
                    assert_eq!(command, HANDSHAKE);
                    self.replies.extend(HANDSHAKE);
                    self.handshaken = true;
                    continue;
                }
                let commit = command[..3] == [2, 3, 8];
                self.commits += usize::from(commit);
                if self.ack_mode == AckMode::Commands || command[..3] == [2, 3, 7] || commit {
                    self.replies
                        .extend(if self.reject { [0x15, command[1]] } else { ACK });
                }
            }
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn partial_io_checks_both_ack_modes_and_stops_after_rejection() {
        for ack_mode in [AckMode::Commands, AckMode::Segments] {
            let peer = Peer {
                pending: Vec::new(),
                replies: VecDeque::new(),
                ack_mode,
                handshaken: false,
                commits: 0,
                reject: false,
            };
            let mut sink = RawfbSink::connect(peer, ack_mode, Duration::from_secs(1)).unwrap();
            sink.present(&Frame::default()).unwrap();
            sink.present(&Frame::default()).unwrap();
            assert_eq!(sink.port.commits, 200);
            assert_eq!(sink.stats.frames, 2);
            assert!(sink.port.replies.is_empty());
            let mut sink = RawfbSink::resume(sink.port, ack_mode, Duration::from_secs(1));
            sink.present(&Frame::default()).unwrap();
            assert_eq!(
                sink.port.commits, 300,
                "confirmed N sessions can resume without another handshake"
            );
            sink.port.reject = true;
            assert!(sink.present(&Frame::default()).is_err());
            let before = sink.stats.wire_bytes;
            assert!(sink.present(&Frame::default()).is_err());
            assert_eq!(
                sink.stats.wire_bytes, before,
                "failed sessions must not send more pixels"
            );
        }
    }
}
