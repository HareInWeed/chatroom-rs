use std::{
  collections::BTreeMap,
  net::SocketAddr,
  result::Result,
  sync::{atomic, Arc},
  time::Duration,
};

use thiserror::Error as ThisError;

use tokio::{net::UdpSocket, sync, task, time};

use parking_lot::Mutex;

use serde::{Deserialize, Serialize};

use bincode::Options;

use byteorder::{ByteOrder, NetworkEndian};

#[derive(Debug)]
pub struct Connection<Coder>
where
  Coder: Options + Copy,
{
  // TODO: use a flatten BtreeMap
  pending_works: Arc<Mutex<BTreeMap<SocketAddr, BTreeMap<u16, sync::oneshot::Sender<Vec<u8>>>>>>,
  counters: Arc<Mutex<BTreeMap<SocketAddr, atomic::AtomicU16>>>,
  sock: Arc<UdpSocket>,
  listener: task::JoinHandle<()>,
  coder: Coder,
  timeout: Duration,
  retry_limits: u32,
}

impl<Coder: 'static + Options + Copy + Send> Connection<Coder> {
  pub fn new(
    sock: UdpSocket,
    coder: Coder,
    timeout: Duration,
    retry_limits: u32,
  ) -> (Self, sync::mpsc::Receiver<(Vec<u8>, SocketAddr)>) {
    let sock = Arc::new(sock);
    let pending_works = Arc::new(Mutex::new(BTreeMap::<
      SocketAddr,
      BTreeMap<u16, sync::oneshot::Sender<Vec<u8>>>,
    >::new()));

    let (sender, receiver) = sync::mpsc::channel::<(Vec<u8>, SocketAddr)>(100);

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
          let id = NetworkEndian::read_u16(&buf[..]);
          let data = buf[2..len].to_vec();
          if id != 0 {
            let mut pending_works = pending_works.lock();
            if let Some(pending_works) = pending_works.get_mut(&addr) {
              if let Some(sender) = pending_works.remove(&id) {
                if let Err(_) = sender.send(data) {
                  // TODO: log error
                }
                continue;
              }
            }
          }
          if let Err(_) = sender.send((data, addr)).await {
            // TODO: log error
          }
        }
      }
    });

    (
      Self {
        pending_works,
        listener,
        sock,
        coder,
        timeout,
        retry_limits,
        counters: Default::default(),
      },
      receiver,
    )
  }

  pub async fn request<Req, Res>(&self, req: &Req, addr: SocketAddr) -> Result<Res, Error>
  where
    Req: Serialize,
    Res: for<'de> Deserialize<'de>,
  {
    let mut buf = vec![0u8, 2];
    let id = self.get_unique_id(addr);
    NetworkEndian::write_u16(&mut buf[..], id);

    self.coder.serialize_into(&mut buf, req)?;

    let mut retry_counter = self.retry_limits;

    loop {
      self.sock.send_to(&buf, addr).await?;

      let (tx, rx) = sync::oneshot::channel::<Vec<u8>>();
      self
        .pending_works
        .lock()
        .entry(addr)
        .or_insert(Default::default())
        .insert(id, tx);

      match time::timeout(self.timeout, rx).await {
        Ok(buf) => {
          let buf = buf?;
          return Ok(self.coder.deserialize::<Res>(&buf)?);
        }
        Err(err) => {
          retry_counter -= 1;
          self.pending_works.lock().remove(&addr);
          if retry_counter == 0 {
            return Err(err.into());
          }
        }
      };
    }
  }

  pub fn get_unique_id(&self, addr: SocketAddr) -> u16 {
    let mut counters = self.counters.lock();
    counters
      .entry(addr)
      .or_insert_with(|| atomic::AtomicU16::new(1))
      .fetch_add(1, atomic::Ordering::SeqCst)
  }

  pub fn release(&self, addr: SocketAddr) -> bool {
    let mut counters = self.counters.lock();
    if counters.remove(&addr).is_some() {
      self.pending_works.lock().remove(&addr);
      true
    } else {
      false
    }
  }

  pub async fn send_to_raw(&self, buf: &[u8], addr: SocketAddr) -> Result<usize, Error> {
    Ok(self.sock.send_to(buf, addr).await?)
  }

  pub async fn send_to<T>(&self, data: &T, addr: SocketAddr) -> Result<usize, Error>
  where
    T: Serialize,
  {
    let buf = self.coder.serialize(data)?;
    Ok(self.sock.send_to(&buf, addr).await?)
  }

  pub async fn send_to_with_meta<T>(&self, data: &T, addr: SocketAddr) -> Result<usize, Error>
  where
    T: Serialize,
  {
    let mut buf = vec![0u8; 2];
    self.coder.serialize_into(&mut buf, data)?;
    Ok(self.sock.send_to(&buf, addr).await?)
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
