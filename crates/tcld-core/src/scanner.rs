use crate::error::{CodeScannerError, DataError, DecoderError, Result};
use std::ops::Range;

pub trait DecodableObject: Sized {
    fn decode(scanner: &mut CodeScanner<'_>) -> Result<Self>;
}

pub struct CodeScanner<'a> {
    bytes: &'a [u8],
    position: usize,
    pub consumed: usize,
}

impl<'a> CodeScanner<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            position: 0,
            consumed: 0,
        }
    }

    pub fn is_at_end(&self) -> bool {
        self.position >= self.bytes.len()
    }

    pub fn remainder(&self) -> String {
        String::from_utf8_lossy(&self.bytes[self.position..]).to_string()
    }

    pub fn match_str(&mut self, s: &str) -> bool {
        let target = s.as_bytes();
        if self.position + target.len() > self.bytes.len() {
            return false;
        }
        if &self.bytes[self.position..self.position + target.len()] != target {
            return false;
        }
        self.position += target.len();
        self.consumed += target.len();
        true
    }

    pub fn match_char(&mut self, ch: u8) -> bool {
        matches!(self.scan_char(), Ok(c) if c == ch)
    }

    pub fn eat_whitespace(&mut self) {
        let _ = self.scan_while_slice(|b| b.is_ascii_whitespace());
    }

    pub fn scan_char(&mut self) -> Result<u8> {
        if self.is_at_end() {
            return Err(CodeScannerError::eof().into());
        }
        let c = self.bytes[self.position];
        self.position += 1;
        self.consumed += 1;
        Ok(c)
    }

    pub fn scan_int(&mut self) -> Result<i32> {
        self.eat_whitespace();
        let s = self
            .scan_while_slice(|b| b == b'-' || b == b'+' || b.is_ascii_digit())
            .ok_or_else(|| {
                if std::env::var("TCLD_DEBUG_SCANNER").is_ok() {
                    let preview = String::from_utf8_lossy(
                        &self.bytes[self.position..self.bytes.len().min(self.position + 64)],
                    );
                    eprintln!(
                        "scan_int failed at position {}: next bytes = {:?}",
                        self.position, preview
                    );
                }
                CodeScannerError::invalid_int()
            })?;
        std::str::from_utf8(s)
            .map_err(|_| CodeScannerError::invalid_int())?
            .parse::<i32>()
            .map_err(|_| CodeScannerError::invalid_int().into())
    }

    pub fn scan_float(&mut self) -> Result<f32> {
        self.eat_whitespace();
        let s = self
            .scan_while_slice(|b| b == b'-' || b == b'+' || b.is_ascii_digit() || b == b'.')
            .ok_or(CodeScannerError::InvalidFloat)?;
        std::str::from_utf8(s)
            .map_err(|_| CodeScannerError::InvalidFloat)?
            .parse::<f32>()
            .map_err(|_| CodeScannerError::InvalidFloat.into())
    }

    pub fn scan_f64(&mut self) -> Result<f64> {
        self.eat_whitespace();
        let s = self
            .scan_while_slice(|b| b == b'-' || b == b'+' || b.is_ascii_digit() || b == b'.')
            .ok_or(CodeScannerError::InvalidFloat)?;
        std::str::from_utf8(s)
            .map_err(|_| CodeScannerError::InvalidFloat)?
            .parse::<f64>()
            .map_err(|_| CodeScannerError::InvalidFloat.into())
    }

    pub fn scan_line(&mut self) -> Result<String> {
        self.scan_line_slice()
            .map(|s| String::from_utf8_lossy(s).to_string())
            .ok_or(CodeScannerError::eof().into())
    }

    pub fn scan_line_slice(&mut self) -> Option<&'a [u8]> {
        self.scan_up_to_slice(|b| b == b'\n' || b == b'\r')
    }

    pub fn scan_word(&mut self) -> Result<String> {
        self.scan_word_slice()
            .map(|s| String::from_utf8_lossy(s).to_string())
            .ok_or(CodeScannerError::eof().into())
    }

    pub fn scan_word_slice(&mut self) -> Option<&'a [u8]> {
        self.scan_up_to_slice(|b| b.is_ascii_whitespace())
    }

    pub fn scan_up_to<F>(&mut self, predicate: F) -> Option<String>
    where
        F: Fn(u8) -> bool,
    {
        self.scan_up_to_slice(predicate)
            .map(|s| String::from_utf8_lossy(s).to_string())
    }

    pub fn scan_up_to_slice<F>(&mut self, predicate: F) -> Option<&'a [u8]>
    where
        F: Fn(u8) -> bool,
    {
        self.scan_while_slice(|b| !predicate(b))
    }

    pub fn scan_while<F>(&mut self, predicate: F) -> Option<String>
    where
        F: Fn(u8) -> bool,
    {
        self.scan_while_slice(predicate)
            .map(|s| String::from_utf8_lossy(s).to_string())
    }

    pub fn scan_while_slice<F>(&mut self, predicate: F) -> Option<&'a [u8]>
    where
        F: Fn(u8) -> bool,
    {
        let start = self.position;
        while self.position < self.bytes.len() && predicate(self.bytes[self.position]) {
            self.position += 1;
        }
        let consumed = self.position - start;
        if consumed == 0 {
            return None;
        }
        self.consumed += consumed;
        Some(&self.bytes[start..self.position])
    }

    pub fn scan_len(&mut self, length: usize) -> Result<String> {
        if self.position + length > self.bytes.len() {
            return Err(CodeScannerError::eof().into());
        }
        let start = self.position;
        self.position += length;
        self.consumed += length;
        Ok(String::from_utf8_lossy(&self.bytes[start..self.position]).to_string())
    }

    pub fn decode_string(&mut self) -> Result<String> {
        self.eat_whitespace();
        let length = self.scan_int()? as usize;
        let data = self.decode_a85(length)?;
        let s = String::from_utf8_lossy(&data).to_string();
        Ok(s)
    }

    pub fn decode_data(&mut self, expected_length: Option<usize>) -> Result<Vec<u8>> {
        self.eat_whitespace();
        let length = self.scan_int()? as usize;
        let expected = expected_length.unwrap_or(length);
        if length != expected {
            return Err(DecoderError::InconsistentLength.into());
        }
        self.decode_a85(length)
    }

    pub fn decode_ranges(
        &mut self,
        count: usize,
        delta_size: i32,
        length_size: i32,
    ) -> Result<Option<Vec<Range<i32>>>> {
        let mut deltas: Option<Vec<u8>> = None;
        let mut lengths: Option<Vec<u8>> = None;

        if delta_size > 0 {
            deltas = Some(self.decode_data(Some(delta_size as usize))?);
        }
        if length_size > 0 {
            lengths = Some(self.decode_data(Some(length_size as usize))?);
        }

        if let (Some(delta_bytes), Some(length_bytes)) = (deltas, lengths) {
            if count == 0 {
                return Ok(None);
            }
            let mut delta_scanner = DataScanner::new(&delta_bytes, ByteOrder::BigEndian);
            let mut length_scanner = DataScanner::new(&length_bytes, ByteOrder::BigEndian);

            let mut offset: i32 = 0;
            let mut ranges: Vec<Range<i32>> = Vec::with_capacity(count);

            for _ in 0..count {
                let mut delta = delta_scanner.scan_u8()? as i32;
                if delta == 0xff {
                    delta = delta_scanner.scan_i32()?;
                }
                offset += delta;
                let mut length = length_scanner.scan_u8()? as i32;
                if length == 0xff {
                    length = length_scanner.scan_i32()?;
                }
                ranges.push(offset..(offset + length));
            }
            return Ok(Some(ranges));
        }

        Ok(None)
    }

    pub fn decode_object<T: DecodableObject>(&mut self) -> Result<T> {
        T::decode(self)
    }

    pub fn decode_array<T: DecodableObject>(&mut self, count: Option<usize>) -> Result<Vec<T>> {
        self.eat_whitespace();
        let read_count = self.scan_int()? as usize;
        let expected = count.unwrap_or(read_count);
        if read_count != expected {
            return Err(CodeScannerError::invalid_input().into());
        }
        let mut values = Vec::with_capacity(read_count);
        for _ in 0..read_count {
            self.eat_whitespace();
            values.push(self.decode_object()?);
        }
        Ok(values)
    }

    fn decode_a85(&mut self, count: usize) -> Result<Vec<u8>> {
        let mut data = Vec::with_capacity(count);
        let mut decode_buf = [0i64; 5];
        let mut current_char: usize = 0;
        let mut bytes_to_decode = count as i32;

        while bytes_to_decode > 0 {
            let ch = self.scan_char()? as usize;
            let code = decode_map(ch as u8);
            match code {
                A85_ILLEGAL_CHAR => return Err(DecoderError::IllegalCharacter.into()),
                A85_WHITESPACE => continue,
                A85_Z => {
                    if current_char != 0 {
                        return Err(DecoderError::MalformedBytes.into());
                    }
                    data.extend_from_slice(&[0, 0, 0, 0]);
                    bytes_to_decode -= 4;
                }
                _ => {
                    decode_buf[current_char] = code as i64;
                    current_char += 1;

                    if bytes_to_decode >= 4 {
                        if current_char > 4 {
                            let mut decode_word = decode_buf[4];
                            for i in (0..4).rev() {
                                decode_word = times85(decode_word) + decode_buf[i];
                            }
                            let word = decode_word as u32;
                            data.push((word & 0xff) as u8);
                            data.push(((word >> 8) & 0xff) as u8);
                            data.push(((word >> 16) & 0xff) as u8);
                            data.push(((word >> 24) & 0xff) as u8);

                            current_char = 0;
                            bytes_to_decode -= 4;
                        }
                    } else if current_char > bytes_to_decode as usize {
                        for buf in decode_buf.iter_mut().skip(bytes_to_decode as usize + 1) {
                            *buf = 0;
                        }
                        let mut decode_word = decode_buf[4];
                        for i in (0..4).rev() {
                            decode_word = times85(decode_word) + decode_buf[i];
                        }
                        let word = decode_word as u32;
                        data.push((word & 0xff) as u8);
                        if bytes_to_decode > 1 {
                            data.push(((word >> 8) & 0xff) as u8);
                        }
                        if bytes_to_decode > 2 {
                            data.push(((word >> 16) & 0xff) as u8);
                        }
                        current_char = 0;
                        bytes_to_decode = 0;
                    }
                }
            }
        }

        Ok(data)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}

pub struct DataScanner<'a> {
    data: &'a [u8],
    position: usize,
    byte_order: ByteOrder,
}

impl<'a> DataScanner<'a> {
    pub fn new(data: &'a [u8], byte_order: ByteOrder) -> Self {
        Self {
            data,
            position: 0,
            byte_order,
        }
    }

    pub fn remaining_size(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }

    pub fn at_end(&self) -> bool {
        self.position >= self.data.len()
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn scan_u8(&mut self) -> Result<u8> {
        self.scan_byte()
    }

    pub fn scan_i8(&mut self) -> Result<i8> {
        Ok(self.scan_byte()? as i8)
    }

    pub fn scan_u16(&mut self) -> Result<u16> {
        let bytes = self.scan_bytes(2)?;
        Ok(match self.byte_order {
            ByteOrder::BigEndian => u16::from_be_bytes([bytes[0], bytes[1]]),
            ByteOrder::LittleEndian => u16::from_le_bytes([bytes[0], bytes[1]]),
        })
    }

    pub fn scan_i16(&mut self) -> Result<i16> {
        Ok(self.scan_u16()? as i16)
    }

    pub fn scan_u32(&mut self) -> Result<u32> {
        let bytes = self.scan_bytes(4)?;
        Ok(match self.byte_order {
            ByteOrder::BigEndian => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            ByteOrder::LittleEndian => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        })
    }

    pub fn scan_i32(&mut self) -> Result<i32> {
        Ok(self.scan_u32()? as i32)
    }

    pub fn scan_u64(&mut self) -> Result<u64> {
        let bytes = self.scan_bytes(8)?;
        Ok(match self.byte_order {
            ByteOrder::BigEndian => u64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
            ByteOrder::LittleEndian => u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
        })
    }

    pub fn scan_i64(&mut self) -> Result<i64> {
        Ok(self.scan_u64()? as i64)
    }

    pub fn scan_bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        if self.remaining_size() < count {
            return Err(DataError::out_of_bounds().into());
        }
        let start = self.position;
        self.position += count;
        Ok(&self.data[start..start + count])
    }

    pub fn scan_byte(&mut self) -> Result<u8> {
        if self.remaining_size() < 1 {
            return Err(DataError::out_of_bounds().into());
        }
        let b = self.data[self.position];
        self.position += 1;
        Ok(b)
    }
}

fn times85(x: i64) -> i64 {
    (((((x << 2) + x) << 2) + x) << 2) + x
}

const A85_WHITESPACE: i32 = -1;
const A85_ILLEGAL_CHAR: i32 = -2;
const A85_Z: i32 = -3;

fn decode_map(ch: u8) -> i32 {
    // table for ASCII 0..127
    const MAP: [i32; 128] = [
        -2, -2, -2, -2, -2, -2, -2, -2, -2, -1, -1, -2, -2, -1, -2, -2, -2, -2, -2, -2, -2, -2, -2,
        -2, -2, -2, -2, -2, -2, -2, -2, -2, -1, 0, -2, 2, -2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
        15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37,
        38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, -2, -2, -2,
        61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83,
        84, 1, 3, 58, 59, -3, -2, 60, -2, -2, -2,
    ];
    if ch as usize >= MAP.len() {
        return A85_ILLEGAL_CHAR;
    }
    MAP[ch as usize]
}
