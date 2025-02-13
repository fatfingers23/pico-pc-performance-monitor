///Based off of my favorite implementation of handling strings and formatting from rp2040-panic-usb-boot
///https://github.com/jannic/rp2040-panic-usb-boot/blob/3c83bab22c12c51458a571642d9a214901f5b60e/src/lib.rs#L11

pub struct Cursor<'a> {
    pub buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a mut [u8]) -> Cursor<'a> {
        Cursor { buf, pos: 0 }
    }

    pub fn clear(&mut self) {
        //empty the buffer
        for i in 0..self.buf.len() {
            self.buf[i] = 0;
        }
        self.pos = 0;
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.pos]).unwrap()
    }
}

impl core::fmt::Write for Cursor<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let len = s.len();
        if len < self.buf.len() - self.pos {
            self.buf[self.pos..self.pos + len].clone_from_slice(s.as_bytes());
            self.pos += len;
            Ok(())
        } else {
            Err(core::fmt::Error)
        }
    }
}
