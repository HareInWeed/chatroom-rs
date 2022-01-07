use std::{collections::BTreeMap, net::SocketAddr, result::Result, sync::Arc, time::Duration};

use thiserror::Error as ThisError;

use tokio::{net::UdpSocket, sync, task, time};

use parking_lot::Mutex;

use serde::{Deserialize, Serialize};

use bincode::Options;

pub struct Connection<Coder>
where
  Coder: Options + Copy,
{
  // TODO: eliminate clone of the buffer
  pending_works: Arc<Mutex<BTreeMap<SocketAddr, sync::oneshot::Sender<Vec<u8>>>>>,
  sock: Arc<UdpSocket>,
  listener: task::JoinHandle<()>,
  coder: Coder,
  receiver: Arc<Mutex<sync::mpsc::Receiver<Vec<u8>>>>,
  timeout: Duration,
  retry_limits: u32,
}

impl<Coder: 'static + Options + Copy + Send> Connection<Coder> {
  pub fn new(sock: UdpSocket, coder: Coder, timeout: Duration, retry_limits: u32) -> Self {
    let sock = Arc::new(sock);
    let pending_works = Arc::new(Mutex::new(BTreeMap::<
      SocketAddr,
      sync::oneshot::Sender<Vec<u8>>,
    >::new()));

    let (sender, receiver) = sync::mpsc::channel::<Vec<u8>>(100);

    let listener = tokio::spawn({
      let sock = sock.clone();
      let pending_works = pending_works.clone();
      async move {
        let mut buf = vec![0; 65535];
        loop {
          let (len, addr) = match sock.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(_) => continue, // TODO: log error
          };
          let data = buf[..len].to_vec();
          let work = pending_works.lock().remove(&addr);
          if let Some(sender) = work {
            if let Err(_) = sender.send(data) {
              continue; // TODO: log error
            }
          } else {
            if let Err(_) = sender.send(data).await {
              continue; // TODO: log error
            }
          }
        }
      }
    });

    Self {
      pending_works,
      listener,
      sock,
      coder,
      timeout,
      retry_limits,
      receiver: Arc::new(Mutex::new(receiver)),
    }
  }

  pub async fn request<Req, Res>(&self, req: &Req, addr: SocketAddr) -> Result<Res, Error>
  where
    Req: Serialize,
    Res: for<'de> Deserialize<'de>,
  {
    let buf = self.coder.serialize(req)?;

    let mut counter = self.retry_limits;

    loop {
      self.sock.send_to(&buf, addr).await?;

      let (tx, rx) = sync::oneshot::channel::<Vec<u8>>();
      self.pending_works.lock().insert(addr.clone(), tx);

      match time::timeout(self.timeout, rx).await {
        Ok(buf) => {
          let buf = buf?;
          return Ok(self.coder.deserialize::<Res>(&buf)?);
        }
        Err(err) => {
          counter -= 1;
          self.pending_works.lock().remove(&addr);
          if counter == 0 {
            return Err(err.into());
          }
        }
      };
    }
  }

  pub async fn send<T>(&self, data: &T) -> Result<usize, Error>
  where
    T: Serialize,
  {
    let buf = self.coder.serialize(data)?;
    Ok(self.sock.send(&buf).await?)
  }

  pub async fn recv<T>(&self) -> Result<T, Error>
  where
    T: for<'de> Deserialize<'de>,
  {
    if let Some(data) = self.receiver.lock().recv().await {
      Ok(self.coder.deserialize::<T>(&data)?)
    } else {
      Err(Error::MpscClosed)
    }
  }
}

impl<Coder: Options + Copy> Drop for Connection<Coder> {
  fn drop(&mut self) {
    self.listener.abort();
  }
}

#[derive(ThisError, Debug)]
pub enum Error {
  #[error(transparent)]
  Network(#[from] std::io::Error),
  #[error(transparent)]
  Timeout(#[from] time::error::Elapsed),
  #[error(transparent)]
  CorruptedData(#[from] bincode::Error),
  #[error(transparent)]
  OneShotReceiveError(#[from] sync::oneshot::error::RecvError),
  #[error("mpsc channel closed")]
  MpscClosed,
}
