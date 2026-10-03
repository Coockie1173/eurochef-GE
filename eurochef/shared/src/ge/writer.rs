//! A growing big-endian buffer with the relative pointers EngineX files are made of.

pub struct Writer {
    pub buf: Vec<u8>,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: vec![] }
    }

    pub fn pos(&self) -> usize {
        self.buf.len()
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn f32s(&mut self, v: &[f32]) {
        for f in v {
            self.f32(*f);
        }
    }

    pub fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }

    pub fn zeros(&mut self, count: usize) {
        self.buf.resize(self.buf.len() + count, 0);
    }

    /// Pads with zeros up to a multiple of `to`
    pub fn align(&mut self, to: usize) {
        let rem = self.buf.len() % to;
        if rem != 0 {
            self.zeros(to - rem);
        }
    }

    /// Leaves room for a relative pointer and returns where it is, for `point`
    pub fn rel(&mut self) -> usize {
        let at = self.pos();
        self.u32(0);
        at
    }

    /// Makes the relative pointer at `at` point to `target` (an offset from the pointer itself)
    pub fn point(&mut self, at: usize, target: usize) {
        self.set_u32(at, (target as i64 - at as i64) as u32);
    }

    /// Makes the relative pointer at `at` point to where the buffer ends now
    pub fn point_here(&mut self, at: usize) {
        self.point(at, self.pos());
    }

    pub fn set_u32(&mut self, at: usize, v: u32) {
        self.buf[at..at + 4].copy_from_slice(&v.to_be_bytes());
    }

    pub fn set_u16(&mut self, at: usize, v: u16) {
        self.buf[at..at + 2].copy_from_slice(&v.to_be_bytes());
    }
}

pub fn read_u32(d: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(d[at..at + 4].try_into().unwrap())
}

pub fn read_i32(d: &[u8], at: usize) -> i32 {
    read_u32(d, at) as i32
}

pub fn read_u16(d: &[u8], at: usize) -> u16 {
    u16::from_be_bytes(d[at..at + 2].try_into().unwrap())
}

pub fn read_f32(d: &[u8], at: usize) -> f32 {
    f32::from_bits(read_u32(d, at))
}

/// Where the relative pointer at `at` points to. None for a null pointer or one outside the data
pub fn read_rel(d: &[u8], at: usize) -> Option<usize> {
    if at + 4 > d.len() {
        return None;
    }
    let rel = read_i32(d, at);
    if rel == 0 {
        return None;
    }
    let target = at as i64 + rel as i64;
    (target >= 0 && (target as usize) < d.len()).then_some(target as usize)
}
