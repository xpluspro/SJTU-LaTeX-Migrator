use serde::Serialize;
use std::io::{self, Read};
use std::time::{Duration, Instant};
use tauri::Emitter;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferProgress<'a> {
    batch_id: &'a str,
    id: usize,
    project: &'a str,
    stage: &'static str,
    bytes: u64,
    total_bytes: Option<u64>,
}

pub(super) fn emit<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    ctx: &super::queue::Context,
    project: &str,
    stage: &'static str,
    bytes: u64,
    total_bytes: Option<u64>,
) {
    let _ = app.emit_to(
        "main",
        "transfer-progress",
        TransferProgress {
            batch_id: &ctx.batch_id,
            id: ctx.id,
            project,
            stage,
            bytes,
            total_bytes,
        },
    );
}

// Always publish the initial and final counts, even for fast transfers. A missing
// Content-Length remains unknown at EOF: the byte count doesn't predict a total.
pub(super) struct Progress<F> {
    bytes: u64,
    total: Option<u64>,
    last_bytes: u64,
    last_update: Instant,
    report: F,
}

impl<F: FnMut(u64, Option<u64>)> Progress<F> {
    pub(super) fn new(total: Option<u64>, mut report: F) -> Self {
        report(0, total);
        Self {
            bytes: 0,
            total,
            last_bytes: 0,
            last_update: Instant::now(),
            report,
        }
    }

    pub(super) fn advance(&mut self, count: usize) {
        self.bytes += count as u64;
        if Some(self.bytes) == self.total
            || self.last_update.elapsed() >= Duration::from_millis(150)
        {
            self.finish();
        }
    }

    pub(super) fn finish(&mut self) {
        if self.last_bytes != self.bytes {
            (self.report)(self.bytes, self.total);
            self.last_bytes = self.bytes;
            self.last_update = Instant::now();
        }
    }

    pub(super) fn bytes(&self) -> u64 {
        self.bytes
    }
}

pub(super) struct GatedReader<T> {
    pub(super) inner: T,
    pub(super) ctx: super::queue::Context,
}
impl<T: Read> Read for GatedReader<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.ctx.checkpoint();
        self.inner.read(buf)
    }
}

pub(super) struct UploadReader<T, F> {
    inner: io::Take<T>,
    progress: Progress<F>,
}

impl<T: Read, F: FnMut(u64, Option<u64>)> UploadReader<T, F> {
    pub(super) fn new(inner: T, length: u64, report: F) -> Self {
        Self {
            inner: inner.take(length),
            progress: Progress::new(Some(length), report),
        }
    }
}

impl<T: Read, F: FnMut(u64, Option<u64>)> Read for UploadReader<T, F> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n != 0 {
            self.progress.advance(n);
        } else if !buf.is_empty() && self.inner.limit() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "ZIP 文件提前结束",
            ));
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_length_stays_unknown_at_eof() {
        let mut samples = Vec::new();
        let mut progress = Progress::new(None, |count, total| samples.push((count, total)));
        progress.advance(3);
        progress.finish();
        progress.finish();
        assert_eq!(samples, [(0, None), (3, None)]);
    }

    #[test]
    fn upload_reports_last_read_without_waiting_for_an_eof_read() {
        let mut samples = Vec::new();
        let mut reader = UploadReader::new(&b"abc"[..], 3, |n, total| samples.push((n, total)));
        let mut buf = [0; 3];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"abc");
        assert_eq!(samples, [(0, Some(3)), (3, Some(3))]);
    }

    #[test]
    fn truncated_file_never_reports_all_bytes_read() {
        let mut last = 0;
        let mut reader = UploadReader::new(&b"ab"[..], 3, |n, _| last = n);
        assert_eq!(
            reader.read_to_end(&mut Vec::new()).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert!(last < 3);
    }
}
