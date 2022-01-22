use std::io::Write;
use tauri::{AppHandle, Manager};
use tracing_subscriber::fmt::MakeWriter;

pub struct LogWriter {
  app: AppHandle,
}

impl Write for LogWriter {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    let _ = self.app.emit_all("log", String::from_utf8_lossy(buf));
    Ok(buf.len())
  }

  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}

pub struct LogWriterMaker {
  app: AppHandle,
}

impl LogWriterMaker {
  pub fn new(app: AppHandle) -> Self {
    Self { app }
  }
}

impl<'a> MakeWriter<'a> for LogWriterMaker {
  type Writer = LogWriter;

  fn make_writer(&self) -> Self::Writer {
    LogWriter {
      app: self.app.clone(),
    }
  }
}
