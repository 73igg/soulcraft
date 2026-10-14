use crate::{Error, Result};

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }
    pub fn require(&self, count: usize) -> Result<()> {
        if count > self.remaining() { Err(Error(format!("truncated chunk at byte {} (needs {count} bytes)", self.position))) } else { Ok(()) }
    }
    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self.position.checked_add(count).ok_or_else(|| Error("byte offset overflow".into()))?;
        let out = self.bytes.get(self.position..end).ok_or_else(|| Error(format!("truncated chunk at byte {}", self.position)))?;
        self.position = end;
        Ok(out)
    }
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| Error("truncated field".into()))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    pub fn raw_floats<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut out = [0.; N];
        for value in &mut out {
            *value = f32::from_le_bytes(self.array()?);
        }
        Ok(out)
    }
    pub fn name(&mut self) -> Result<String> {
        let size = self.u32()? as usize;
        if size > 16_384 {
            return Err(Error("name exceeds 16 KiB".into()));
        }
        let bytes = self.take(size)?;
        let bytes = bytes.split(|b| *b == 0).next().unwrap_or_default();
        match std::str::from_utf8(bytes) {
            Ok(text) => Ok(text.into()),
            Err(_) => Ok(encoding_rs::GBK.decode(bytes).0.into_owned()),
        }
    }
}
