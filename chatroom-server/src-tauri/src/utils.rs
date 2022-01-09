macro_rules! log {
  ($app_handle:expr, $format:literal $(,)?) => {
    {
        use std::fmt::Write;
        use tauri::Manager;
        use time;
        let mut msg = String::new();
        let now = time::OffsetDateTime::now_utc();
        let now = match time::UtcOffset::current_local_offset() {
          Ok(offset) => now.to_offset(offset),
          Err(_) => now,
        };
        let _ = write!(&mut msg, $format, timestamp=now);
        let _ = ($app_handle).emit_all("log", msg);
      }
  };
  ($app_handle:expr, $format:literal, $($args:expr),+ $(,)?) => {
    {
      use std::fmt::Write;
      use tauri::Manager;
      use time;
      let mut msg = String::new();
      let now = time::OffsetDateTime::now_utc();
      let now = match time::UtcOffset::current_local_offset() {
        Ok(offset) => now.to_offset(offset),
        Err(_) => now,
      };
      let _ = write!(&mut msg, $format, $($args),+, timestamp=now);
      let _ = ($app_handle).emit_all("log", msg);
    }
  };
}

pub(crate) use log;
