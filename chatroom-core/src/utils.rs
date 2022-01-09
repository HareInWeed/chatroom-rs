use thiserror::Error as ThisError;

use std::{fmt::Display, future::Future, time::Duration};

use bincode;

use serde::{Deserialize, Serialize};

pub struct SeqDisplay<'a, T: Display>(pub &'a [T]);

impl<'a, T: Display> Display for SeqDisplay<'a, T> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let width = f.width().unwrap_or(0);
    let delimiter = f.fill();
    let mut iter = self.0.iter();
    if let Some(x) = iter.next() {
      write!(f, "{:width$}", x, width = width)?;
    }
    for x in iter {
      write!(f, "{}", delimiter)?;
      write!(f, "{:width$}", x, width = width)?;
    }
    Ok(())
  }
}

pub fn default_timeout<T>(fut: T) -> tokio::time::Timeout<T>
where
  T: Future,
{
  tokio::time::timeout(Duration::from_secs(5), fut)
}

#[derive(ThisError, Debug)]
pub enum Error {
  #[error(transparent)]
  Network(#[from] std::io::Error),
  #[error(transparent)]
  StdIO(std::io::Error),
  #[error(transparent)]
  Timeout(#[from] tokio::time::error::Elapsed),
  #[error(transparent)]
  Format(#[from] std::fmt::Error),
  #[error(transparent)]
  Server(#[from] crate::data::ErrorCode),
  #[error(transparent)]
  CorruptedData(#[from] bincode::Error),
  #[error(transparent)]
  Connection(#[from] crate::connection::Error),
  #[error(transparent)]
  InvalidSockAddr(#[from] std::net::AddrParseError),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorMsg {
  msg: String,
}

impl<T: Display> From<T> for ErrorMsg {
  fn from(err: T) -> Self {
    Self {
      msg: err.to_string(),
    }
  }
}
