use aes::cipher::KeyIvInit;
use async_compression::tokio::bufread::ZlibDecoder;
use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};

use crate::{
    Aes128Cfb8Dec, CompressionThreshold, MAX_PACKET_DATA_SIZE, MAX_PACKET_SIZE, PacketDecodeError,
    RawPacket, StreamDecryptor, ser::NetworkReadExt,
};

// decrypt -> decompress -> raw

pub enum DecompressionReader<R: AsyncRead + Unpin> {
    Decompress(ZlibDecoder<BufReader<R>>),
    None(R),
}

impl<R: AsyncRead + Unpin> AsyncRead for DecompressionReader<R> {
    #[inline]
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Decompress(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
            Self::None(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
        }
    }
}

pub enum DecryptionReader<R: AsyncRead + Unpin> {
    Decrypt(Box<StreamDecryptor<R>>),
    None(R),
}

impl<R: AsyncRead + Unpin> DecryptionReader<R> {
    #[must_use]
    pub fn upgrade(self, cipher: Aes128Cfb8Dec) -> Self {
        match self {
            Self::None(stream) => Self::Decrypt(Box::new(StreamDecryptor::new(cipher, stream))),
            Self::Decrypt(_) => self,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for DecryptionReader<R> {
    #[inline]
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Decrypt(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
            Self::None(reader) => {
                let reader = std::pin::Pin::new(reader);
                reader.poll_read(cx, buf)
            }
        }
    }
}

/// Decoder: Client -> Server
/// Supports `ZLib` decoding/decompression
/// Supports Aes128 Encryption
pub struct TCPNetworkDecoder<R: AsyncRead + Unpin> {
    reader: Option<DecryptionReader<R>>,
    compression: Option<CompressionThreshold>,
    payload_scratch: BytesMut,
    frame_length: Option<usize>,
    length_value: usize,
    length_bytes: u32,
    last_read: tokio::time::Instant,
}

impl<R: AsyncRead + Unpin> TCPNetworkDecoder<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: Some(DecryptionReader::None(reader)),
            compression: None,
            payload_scratch: BytesMut::new(),
            frame_length: None,
            length_value: 0,
            length_bytes: 0,
            last_read: tokio::time::Instant::now(),
        }
    }

    pub const fn set_compression(&mut self, threshold: CompressionThreshold) {
        self.compression = Some(threshold);
    }

    /// NOTE: Encryption can only be set; a minecraft stream cannot go back to being unencrypted
    pub fn set_encryption(&mut self, key: &[u8; 16]) -> Result<(), PacketDecodeError> {
        if matches!(self.reader, Some(DecryptionReader::Decrypt(_))) {
            return Err(PacketDecodeError::Message(
                "Encryption already enabled".into(),
            ));
        }
        let cipher = Aes128Cfb8Dec::new_from_slices(key, key)
            .map_err(|_| PacketDecodeError::Message("Invalid key".into()))?;

        if let Some(reader) = self.reader.take() {
            self.reader = Some(reader.upgrade(cipher));
        }
        Ok(())
    }

    pub async fn get_raw_packet(&mut self) -> Result<RawPacket, PacketDecodeError> {
        let reader = self
            .reader
            .as_mut()
            .ok_or_else(|| PacketDecodeError::Message("Reader missing".into()))?;
        // Varint21FrameDecoder.decode retains incomplete frames. State survives timer cancellation.
        while self.frame_length.is_none() {
            // Netty ReadTimeoutHandler measures inactivity between bytes, not total frame duration.
            let byte = tokio::time::timeout_at(
                self.last_read + std::time::Duration::from_secs(30),
                reader.read_u8(),
            )
            .await
            .map_err(|_| PacketDecodeError::ReadTimeout)?
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::UnexpectedEof && self.length_bytes == 0 {
                    PacketDecodeError::ConnectionClosed
                } else {
                    PacketDecodeError::MalformedLength(err.to_string())
                }
            })?;
            self.last_read = tokio::time::Instant::now();
            self.length_value |= usize::from(byte & 0x7f) << (7 * self.length_bytes);
            self.length_bytes += 1;
            if byte & 0x80 == 0 {
                if self.length_value == 0 || self.length_value as u64 > MAX_PACKET_SIZE {
                    return Err(PacketDecodeError::OutOfBounds);
                }
                self.frame_length = Some(self.length_value);
            } else if self.length_bytes == 3 {
                return Err(PacketDecodeError::MalformedLength(
                    "Frame length exceeds 21 bits".into(),
                ));
            }
        }
        let length = self.length_value;
        while self.payload_scratch.len() < length {
            let remaining = length - self.payload_scratch.len();
            let mut bounded = (&mut *reader).take(remaining as u64);
            let read = tokio::time::timeout_at(
                self.last_read + std::time::Duration::from_secs(30),
                bounded.read_buf(&mut self.payload_scratch),
            )
            .await
            .map_err(|_| PacketDecodeError::ReadTimeout)?
            .map_err(|err| PacketDecodeError::Message(err.to_string()))?;
            self.last_read = tokio::time::Instant::now();
            if read == 0 {
                return Err(PacketDecodeError::MalformedLength("Truncated frame".into()));
            }
        }
        let packet = Self::decode_frame(&self.payload_scratch, self.compression).await?;
        self.payload_scratch.clear();
        self.frame_length = None;
        self.length_value = 0;
        self.length_bytes = 0;
        Ok(packet)
    }

    async fn decode_frame(
        mut frame: &[u8],
        compression: Option<CompressionThreshold>,
    ) -> Result<RawPacket, PacketDecodeError> {
        let mut inflated = Vec::new();
        if let Some(threshold) = compression {
            let declared = frame.get_var_int()?.0;
            // CompressionDecoder.decode: zero is raw; only compressed data must meet threshold.
            if declared != 0 {
                let length = usize::try_from(declared).map_err(|_| PacketDecodeError::TooLong)?;
                if length > MAX_PACKET_DATA_SIZE {
                    return Err(PacketDecodeError::TooLong);
                }
                if length < threshold {
                    return Err(PacketDecodeError::NotCompressed);
                }
                ZlibDecoder::new(BufReader::new(frame))
                    .take((length + 1) as u64)
                    .read_to_end(&mut inflated)
                    .await
                    .map_err(|err| PacketDecodeError::FailedDecompression(err.to_string()))?;
                if inflated.len() != length {
                    return Err(PacketDecodeError::FailedDecompression(
                        "Decompressed length mismatch".into(),
                    ));
                }
                frame = &inflated;
            }
        }
        let packet_id = frame
            .get_var_int()
            .map_err(|_| PacketDecodeError::DecodeID)?
            .0;
        if packet_id < 0 {
            return Err(PacketDecodeError::DecodeID);
        }
        Ok(RawPacket {
            id: packet_id,
            payload: Bytes::copy_from_slice(frame),
        })
    }
}

#[cfg(test)]
mod tests {

    use std::io::Write;

    use crate::{VarInt, ser::NetworkWriteExt};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn read_timeout_measures_byte_inactivity_across_partial_frames() {
        use tokio::io::AsyncWriteExt;
        let (read, mut write) = tokio::io::duplex(16);
        let mut decoder = TCPNetworkDecoder::new(read);
        write.write_all(&[2]).await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(20), decoder.get_raw_packet())
                .await
                .is_err()
        );
        write.write_all(&[1]).await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(20), decoder.get_raw_packet())
                .await
                .is_err()
        );
        write.write_all(&[7]).await.unwrap();
        assert_eq!(
            decoder.get_raw_packet().await.unwrap().payload.as_ref(),
            &[7]
        );
        assert!(matches!(
            decoder.get_raw_packet().await,
            Err(PacketDecodeError::ReadTimeout)
        ));
    }
    use aes::Aes128;
    use cfb8::Encryptor as Cfb8Encryptor;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    #[tokio::test]
    async fn compression_threshold_edges() {
        // Frame: raw marker, packet id, two payload bytes. Raw is valid above threshold.
        let mut decoder = TCPNetworkDecoder::new(&[4, 0, 1, 2, 3][..]);
        decoder.set_compression(1);
        assert!(decoder.get_raw_packet().await.is_ok());
        // zlib for [1, 2, 3], declaring a three-byte packet body.
        let fixture = [
            12, 3, 0x78, 0x9c, 0x63, 0x64, 0x62, 0x06, 0x00, 0x00, 0x0d, 0x00, 0x07,
        ];
        let mut at = TCPNetworkDecoder::new(fixture.as_slice());
        at.set_compression(3);
        assert!(at.get_raw_packet().await.is_ok());
        let mut below = TCPNetworkDecoder::new(fixture.as_slice());
        below.set_compression(4);
        assert!(matches!(
            below.get_raw_packet().await,
            Err(PacketDecodeError::NotCompressed)
        ));
        // Declared inflated size 8,388,609 and a negative size, with no zlib payload.
        for fixture in [
            &[4, 0x81, 0x80, 0x80, 0x04][..],
            &[5, 0xff, 0xff, 0xff, 0xff, 0x0f][..],
        ] {
            let mut oversized = TCPNetworkDecoder::new(fixture);
            oversized.set_compression(1);
            assert!(matches!(
                oversized.get_raw_packet().await,
                Err(PacketDecodeError::TooLong)
            ));
        }
    }

    #[tokio::test]
    async fn cancelled_partial_frame_resumes() {
        use tokio::io::AsyncWriteExt;
        let (mut peer, stream) = tokio::io::duplex(16);
        let mut decoder = TCPNetworkDecoder::new(stream);
        peer.write_all(&[0x83]).await.unwrap(); // Incomplete two-byte length.
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(5),
                decoder.get_raw_packet()
            )
            .await
            .is_err()
        );
        peer.write_all(&[0x00, 0x01]).await.unwrap(); // Length three, id one, missing payload.
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(5),
                decoder.get_raw_packet()
            )
            .await
            .is_err()
        );
        peer.write_all(&[0x02, 0x03, 0x01, 0x04]).await.unwrap();
        let first = decoder.get_raw_packet().await.unwrap();
        assert_eq!(first.id, 1);
        assert_eq!(first.payload.as_ref(), &[2, 3]);
        assert_eq!(decoder.get_raw_packet().await.unwrap().id, 4);
    }

    /// Helper function to compress data using libdeflater's Zlib compressor
    fn compress_zlib(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
        let mut compressed = Vec::new();
        ZlibEncoder::new(&mut compressed, Compression::default()).write_all(data)?;
        Ok(compressed)
    }

    /// Helper function to encrypt data using AES-128 CFB-8 mode
    fn encrypt_aes128(
        data: &mut [u8],
        key: &[u8; 16],
        iv: &[u8; 16],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut encryptor =
            Cfb8Encryptor::<Aes128>::new_from_slices(key, iv).map_err(|_| "Invalid key/iv")?;
        encryptor.encrypt(data);
        Ok(())
    }

    /// Helper function to build a packet with optional compression and encryption
    fn build_packet(
        packet_id: i32,
        payload: &[u8],
        compress: bool,
        key: Option<&[u8; 16]>,
        iv: Option<&[u8; 16]>,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut buffer = Vec::new();

        if compress {
            // Create a buffer that includes `packet_id_varint` and payload
            let mut data_to_compress = Vec::new();
            let packet_id_varint = VarInt(packet_id);
            data_to_compress.write_var_int(&packet_id_varint)?;
            data_to_compress.write_slice(payload)?;

            // Compress the combined data
            let compressed_payload = compress_zlib(&data_to_compress)?;
            let data_len = data_to_compress.len() as i32; // 1 + payload.len()
            let data_len_varint = VarInt(data_len);
            buffer.write_var_int(&data_len_varint)?;
            buffer.write_slice(&compressed_payload)?;
        } else {
            // No compression; `data_len` is payload length
            let packet_id_varint = VarInt(packet_id);
            buffer.write_var_int(&packet_id_varint)?;
            buffer.write_slice(payload)?;
        }

        // Calculate packet length: length of buffer
        let packet_len = buffer.len() as i32;
        let packet_len_varint = VarInt(packet_len);
        let mut packet_length_encoded = Vec::new();
        packet_len_varint.encode(&mut packet_length_encoded)?;

        // Create a new buffer for the entire packet
        let mut packet = Vec::new();
        packet.extend_from_slice(&packet_length_encoded);
        packet.extend_from_slice(&buffer);

        // Encrypt if key and IV are provided.
        if let (Some(k), Some(v)) = (key, iv) {
            encrypt_aes128(&mut packet, k, v)?;
        }
        Ok(packet)
    }

    /// Test decoding without compression and encryption
    #[tokio::test]
    async fn decode_without_compression_and_encryption() -> Result<(), Box<dyn std::error::Error>> {
        // Sample packet data: packet_id = 1, payload = "Hello"
        let packet_id = 1;
        let payload = b"Hello";

        // Build the packet without compression and encryption
        let packet = build_packet(packet_id, payload, false, None, None)?;

        // Initialize the decoder without compression and encryption
        let mut decoder = TCPNetworkDecoder::new(packet.as_slice());

        // Attempt to decode
        let raw_packet = decoder.get_raw_packet().await.map_err(|e| e.to_string())?;

        assert_eq!(raw_packet.id, packet_id);
        assert_eq!(raw_packet.payload.as_ref(), payload);
        Ok(())
    }

    /// Test decoding with compression
    #[tokio::test]
    async fn decode_with_compression() -> Result<(), Box<dyn std::error::Error>> {
        // Sample packet data: packet_id = 2, payload = "Hello, compressed world!"
        let packet_id = 2;
        let payload = b"Hello, compressed world!";

        // Build the packet with compression enabled
        let packet = build_packet(packet_id, payload, true, None, None)?;

        // Initialize the decoder with compression enabled
        let mut decoder = TCPNetworkDecoder::new(packet.as_slice());
        // Larger than payload
        decoder.set_compression(1);

        // Attempt to decode
        let raw_packet = decoder.get_raw_packet().await.map_err(|e| e.to_string())?;

        assert_eq!(raw_packet.id, packet_id);
        assert_eq!(raw_packet.payload.as_ref(), payload);
        Ok(())
    }

    /// Test decoding with encryption
    #[tokio::test]
    async fn decode_with_encryption() -> Result<(), Box<dyn std::error::Error>> {
        // Sample packet data: packet_id = 3, payload = "Hello, encrypted world!"
        let packet_id = 3;
        let payload = b"Hello, encrypted world!";

        // Define encryption key and IV
        let key = [0x00u8; 16]; // Example key

        // Build the packet with encryption enabled (no compression)
        let packet = build_packet(packet_id, payload, false, Some(&key), Some(&key))?;

        // Initialize the decoder with encryption enabled
        let mut decoder = TCPNetworkDecoder::new(packet.as_slice());
        decoder.set_encryption(&key).map_err(|e| e.to_string())?;

        // Attempt to decode
        let raw_packet = decoder.get_raw_packet().await.map_err(|e| e.to_string())?;

        assert_eq!(raw_packet.id, packet_id);
        assert_eq!(raw_packet.payload.as_ref(), payload);
        Ok(())
    }

    /// Test decoding with both compression and encryption
    #[tokio::test]
    async fn decode_with_compression_and_encryption() -> Result<(), Box<dyn std::error::Error>> {
        // Sample packet data: packet_id = 4, payload = "Hello, compressed and encrypted world!"
        let packet_id = 4;
        let payload = b"Hello, compressed and encrypted world!";

        // Define encryption key and IV
        let key = [0x01u8; 16]; // Example key
        let iv = [0x01u8; 16]; // Example IV

        // Build the packet with both compression and encryption enabled
        let packet = build_packet(packet_id, payload, true, Some(&key), Some(&iv))?;

        // Initialize the decoder with both compression and encryption enabled
        let mut decoder = TCPNetworkDecoder::new(packet.as_slice());
        decoder.set_compression(1);
        decoder.set_encryption(&key).map_err(|e| e.to_string())?;

        // Attempt to decode
        let raw_packet = decoder.get_raw_packet().await.map_err(|e| e.to_string())?;

        assert_eq!(raw_packet.id, packet_id);
        assert_eq!(raw_packet.payload.as_ref(), payload);
        Ok(())
    }

    /// Test decoding with invalid compressed data
    #[tokio::test]
    async fn decode_with_invalid_compressed_data() -> Result<(), Box<dyn std::error::Error>> {
        // Sample packet data: packet_id = 5, payload_len = 10, but compressed data is invalid
        let data_len = 10; // Expected decompressed size
        let invalid_compressed_data = vec![0xFF, 0xFF, 0xFF]; // Invalid Zlib data

        // Build the packet with compression enabled but invalid compressed data
        let mut buffer = Vec::new();
        let data_len_varint = VarInt(data_len);
        buffer.write_var_int(&data_len_varint)?;
        buffer.write_slice(&invalid_compressed_data)?;

        // Calculate packet length: VarInt(data_len) + invalid compressed data
        let packet_len = buffer.len() as i32;
        let packet_len_varint = VarInt(packet_len);

        // Create a new buffer for the entire packet
        let mut packet_buffer = Vec::new();
        packet_buffer.write_var_int(&packet_len_varint)?;
        packet_buffer.write_slice(&buffer)?;

        let packet_bytes = packet_buffer;

        // Initialize the decoder with compression enabled
        let mut decoder = TCPNetworkDecoder::new(&packet_bytes[..]);
        decoder.set_compression(1);

        // Attempt to decode and expect a decompression error
        let result = decoder.get_raw_packet().await;

        assert!(result.is_err(), "This should have errored!");
        Ok(())
    }

    /// Test decoding with a zero-length packet
    #[tokio::test]
    async fn decode_with_zero_length_packet() -> Result<(), Box<dyn std::error::Error>> {
        // Sample packet data: packet_id = 7, payload = "" (empty)
        let packet_id = 7;
        let payload = b"";

        // Build the packet without compression and encryption
        let packet = build_packet(packet_id, payload, false, None, None)?;

        // Initialize the decoder without compression and encryption
        let mut decoder = TCPNetworkDecoder::new(packet.as_slice());

        // Attempt to decode and expect a read error
        let raw_packet = decoder.get_raw_packet().await.map_err(|e| e.to_string())?;
        assert_eq!(raw_packet.id, packet_id);
        assert_eq!(raw_packet.payload.as_ref(), payload);
        Ok(())
    }

    /// Test decoding with maximum length packet
    #[tokio::test]
    #[expect(clippy::print_stdout)]
    async fn decode_with_maximum_length_packet() -> Result<(), Box<dyn std::error::Error>> {
        // Sample packet data: packet_id = 8, payload = "A" repeated MAX_PACKET_SIZE times
        // Sample packet data: packet_id = 8, payload = "A" repeated (MAX_PACKET_SIZE - 1) times
        let packet_id = 8;
        let payload = vec![0x41u8; MAX_PACKET_SIZE as usize - 1]; // "A" repeated

        // Build the packet with compression enabled
        let packet = build_packet(packet_id, &payload, true, None, None)?;
        println!("Built packet (with compression, maximum length): {packet:?}");

        // Initialize the decoder with compression enabled
        let mut decoder = TCPNetworkDecoder::new(packet.as_slice());
        decoder.set_compression(MAX_PACKET_SIZE as usize);

        // Attempt to decode
        let result = decoder.get_raw_packet().await;

        let raw_packet = result.map_err(|e| e.to_string())?;
        assert_eq!(raw_packet.id, packet_id);
        assert_eq!(raw_packet.payload.as_ref(), payload);
        Ok(())
    }

    /// Test decoding multiple packets sequentially to verify capacity is retained for zero allocation
    #[tokio::test]
    async fn decode_multiple_packets_zero_allocation() -> Result<(), Box<dyn std::error::Error>> {
        let packet1 = build_packet(1, b"Hello", false, None, None)?;
        let packet2 = build_packet(2, b"World", false, None, None)?;

        let mut stream = Vec::new();
        stream.extend_from_slice(&packet1);
        stream.extend_from_slice(&packet2);

        let mut decoder = TCPNetworkDecoder::new(stream.as_slice());

        let p1 = decoder.get_raw_packet().await?;
        assert_eq!(p1.id, 1);
        assert_eq!(p1.payload.as_ref(), b"Hello");
        drop(p1);

        let cap_after_p1 = decoder.payload_scratch.capacity();
        assert!(
            cap_after_p1 > 0,
            "Capacity should be allocated after first read"
        );

        let p2 = decoder.get_raw_packet().await?;
        assert_eq!(p2.id, 2);
        assert_eq!(p2.payload.as_ref(), b"World");
        drop(p2);

        let cap_after_p2 = decoder.payload_scratch.capacity();
        assert_eq!(
            cap_after_p2, cap_after_p1,
            "Buffer capacity should be retained and reused without new heap allocations"
        );
        Ok(())
    }
}
