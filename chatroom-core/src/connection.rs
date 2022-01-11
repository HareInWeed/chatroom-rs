use std::{
  collections::{BTreeMap, HashMap},
  iter,
  net::SocketAddr,
  result::Result,
  sync::{atomic, Arc},
  time::Duration,
};

use thiserror::Error as ThisError;

use tokio::{net::UdpSocket, sync, task, time};

use parking_lot::{Mutex, RwLock};

use serde::{Deserialize, Serialize};

use bincode::Options;

use byteorder::{ByteOrder, NetworkEndian};

use futures::future::try_join_all;

use crate::data::{serialize_with_meta, SecureMsg};

use crypto_box::{aead::Aead, generate_nonce, ChaChaBox, PublicKey, SecretKey};

use rand::{rngs::StdRng, thread_rng, SeedableRng};

struct SecureBox {
  coder: ChaChaBox,
  en_nonce_gen: StdRng,
  de_nonce_gen: StdRng,
}

// TODO: maybe we should merge `SecureConnection` with `Connection`
pub struct SecureConnection<Coder>
where
  Coder: Options + Copy,
{
  sock: Arc<UdpSocket>,
  coder: Coder,
  pub_keys: Arc<RwLock<HashMap<SocketAddr, PublicKey>>>,
  secure_boxes: RwLock<HashMap<SocketAddr, SecureBox>>,
  pub_key_sender: sync::mpsc::Sender<(PublicKey, SocketAddr)>,
  key_response_notifier: sync::Notify,
  secret_key: Mutex<SecretKey>,
}

impl<Coder: 'static + Options + Copy + Send + Sync> SecureConnection<Coder> {
  pub fn new(
    sock: UdpSocket,
    pub_keys: Arc<RwLock<HashMap<SocketAddr, PublicKey>>>,
    coder: Coder,
  ) -> (Self, sync::mpsc::Receiver<(PublicKey, SocketAddr)>) {
    let sock = Arc::new(sock);
    let secret_key = Mutex::new(SecretKey::generate(&mut thread_rng()));
    let (sender, receiver) = sync::mpsc::channel(100);
    let key_response_notifier = sync::Notify::new();
    let connection = Self {
      sock,
      coder,
      pub_key_sender: sender,
      pub_keys,
      secret_key,
      key_response_notifier,
      secure_boxes: Default::default(),
    };
    connection.sync_all_pub_keys();
    (connection, receiver)
  }

  #[inline(always)]
  pub fn get_coder(&self) -> Coder {
    self.coder
  }

  pub async fn recv_from_raw(&self, buf: &mut [u8]) -> Result<(Vec<u8>, SocketAddr), Error> {
    loop {
      let (len, addr) = self.sock.recv_from(buf).await?;
      match self.coder.deserialize::<SecureMsg>(&buf[..len])? {
        key_msg @ (SecureMsg::PeerKey(_) | SecureMsg::MyKey(_)) => {
          let key = match &key_msg {
            SecureMsg::MyKey(key) => key,
            SecureMsg::PeerKey(key) => key,
            _ => unreachable!(),
          };
          let public_key = PublicKey::from(key.clone());
          self.update_pub_keys(iter::once((public_key.clone(), addr)));
          if let Err(_) = self.pub_key_sender.send((public_key.clone(), addr)).await {
            // TODO: log error
          }

          if matches!(key_msg, SecureMsg::PeerKey(_)) {
            self.key_response_notifier.notify_waiters();
          }
          if matches!(key_msg, SecureMsg::MyKey(_)) {
            let msg = SecureMsg::PeerKey(self.get_public_key().as_bytes().clone());
            let buf = self.coder.serialize(&msg)?;
            let sock = self.sock.clone();
            tokio::spawn(async move {
              if let Err(_) = sock.send_to(&buf, addr).await {
                // TODO: log error
              }
            });
          }
        }
        SecureMsg::Msg(ciphertext) => {
          let mut secure_boxes = self.secure_boxes.write();
          if let Some(secure_box) = secure_boxes.get_mut(&addr) {
            let nonce = generate_nonce(&mut secure_box.de_nonce_gen);
            let plain_data = match secure_box.coder.decrypt(&nonce, &ciphertext[..]) {
              Ok(s) => s,
              Err(_) => break Err(Error::DecryptionFailed),
            };
            break Ok((plain_data, addr));
          } else {
            break Err(Error::NoSrcKey);
          }
        }
      }
    }
  }

  pub async fn send_to_raw(&self, buf: &[u8], addr: SocketAddr) -> Result<usize, Error> {
    let encrypted_data = self.secure_serialize(buf, addr)?;
    self.send_to_insecurely(&encrypted_data[..], addr).await
  }

  #[inline(always)]
  async fn send_to_insecurely(&self, buf: &[u8], addr: SocketAddr) -> Result<usize, Error> {
    Ok(self.sock.send_to(buf, addr).await?)
  }

  fn secure_serialize(&self, buf: &[u8], addr: SocketAddr) -> Result<Vec<u8>, Error> {
    let mut secure_boxes = self.secure_boxes.write();
    if let Some(b) = secure_boxes.get_mut(&addr) {
      let nonce = generate_nonce(&mut b.en_nonce_gen);
      let encrypted_data = match b.coder.encrypt(&nonce, buf) {
        Ok(s) => s,
        Err(_) => return Err(Error::EncryptionFailed),
      };
      let secure_msg = SecureMsg::Msg(encrypted_data);
      let msg = self.coder.serialize(&secure_msg)?;
      Ok(msg)
    } else {
      Err(Error::NoDestKey)
    }
  }

  // secret key related
  pub fn refresh_secret_key(&self) {
    *self.secret_key.lock() = SecretKey::generate(&mut thread_rng());
  }

  pub fn get_secret_key(&self) -> SecretKey {
    self.secret_key.lock().clone()
  }

  pub fn get_public_key(&self) -> PublicKey {
    self.secret_key.lock().public_key()
  }

  pub async fn exchange_key_with(&self, addr: SocketAddr) -> Result<(), Error> {
    let msg = SecureMsg::MyKey(self.get_public_key().as_bytes().clone());
    let buf = self.coder.serialize(&msg)?;
    self.send_to_insecurely(&buf, addr).await?;
    // TODO: maybe we should remove this?
    self.key_response_notifier.notified().await;
    Ok(())
  }

  // public keys related
  pub fn update_pub_keys<I>(&self, iter: I)
  where
    I: Iterator<Item = (PublicKey, SocketAddr)>,
  {
    let secret_key = self.secret_key.lock();
    let mut secure_boxes = self.secure_boxes.write();
    let mut pub_keys = self.pub_keys.write();
    let my_key = secret_key.public_key();
    for (key, addr) in iter {
      let coder = ChaChaBox::new(&key, &secret_key);
      let en_gen = StdRng::from_seed(key.as_bytes().clone());
      let de_gen = StdRng::from_seed(my_key.as_bytes().clone());
      secure_boxes.insert(
        addr,
        SecureBox {
          coder,
          en_nonce_gen: en_gen,
          de_nonce_gen: de_gen,
        },
      );
      pub_keys.insert(addr, key);
    }
  }

  pub fn sync_pub_keys<I>(&self, iter: I)
  where
    I: Iterator<Item = SocketAddr>,
  {
    let secret_key = self.secret_key.lock();
    let mut secure_boxes = self.secure_boxes.write();
    let pub_keys = self.pub_keys.read();
    let my_key = secret_key.public_key();
    for addr in iter {
      if let Some(key) = pub_keys.get(&addr) {
        let coder = ChaChaBox::new(key, &secret_key);
        let en_gen = StdRng::from_seed(key.as_bytes().clone());
        let de_gen = StdRng::from_seed(my_key.as_bytes().clone());
        secure_boxes.insert(
          addr,
          SecureBox {
            coder,
            en_nonce_gen: en_gen,
            de_nonce_gen: de_gen,
          },
        );
      }
    }
  }

  pub fn sync_all_pub_keys(&self) {
    let secret_key = self.secret_key.lock();
    let mut secure_boxes = self.secure_boxes.write();
    let my_key = secret_key.public_key();
    for (&addr, key) in self.pub_keys.read().iter() {
      let coder = ChaChaBox::new(&key, &secret_key);
      let en_gen = StdRng::from_seed(key.as_bytes().clone());
      let de_gen = StdRng::from_seed(my_key.as_bytes().clone());
      secure_boxes.insert(
        addr,
        SecureBox {
          coder,
          en_nonce_gen: en_gen,
          de_nonce_gen: de_gen,
        },
      );
    }
  }

  pub fn release(&self, addr: SocketAddr) {
    self.pub_keys.write().remove(&addr);
    self.secure_boxes.write().remove(&addr);
  }

  // recv and send helper
  pub async fn recv_from<T>(&self, buf: &mut [u8]) -> Result<(T, SocketAddr), Error>
  where
    T: for<'de> Deserialize<'de>,
  {
    let (data, addr) = self.recv_from_raw(buf).await?;
    Ok((self.coder.deserialize(&data[..])?, addr))
  }

  pub async fn send_to_multiple_with_meta<T, I>(
    &self,
    data: &T,
    addrs: I,
    id: u16,
  ) -> Result<Vec<usize>, Error>
  where
    T: Serialize,
    I: Iterator<Item = SocketAddr>,
  {
    let buf = serialize_with_meta(self.coder, data, id)?;
    Ok(try_join_all(addrs.map(|addr| self.send_to_raw(&buf, addr))).await?)
  }

  pub async fn send_to_multiple_with_empty_meta<T, I>(
    &self,
    data: &T,
    addrs: I,
  ) -> Result<Vec<usize>, Error>
  where
    T: Serialize,
    I: Iterator<Item = SocketAddr>,
  {
    let mut buf = vec![0u8; 2];
    self.coder.serialize_into(&mut buf, data)?;

    Ok(try_join_all(addrs.map(|addr| self.send_to_raw(&buf, addr))).await?)
  }

  pub async fn send_to<T>(&self, data: &T, addr: SocketAddr) -> Result<usize, Error>
  where
    T: Serialize,
  {
    let buf = self.coder.serialize(data)?;
    Ok(self.send_to_raw(&buf, addr).await?)
  }

  pub async fn send_to_with_meta<T>(
    &self,
    data: &T,
    addr: SocketAddr,
    id: u16,
  ) -> Result<usize, Error>
  where
    T: Serialize,
  {
    let mut buf = vec![0u8; 2];
    NetworkEndian::write_u16(&mut buf[..], id);
    self.coder.serialize_into(&mut buf, data)?;
    Ok(self.send_to_raw(&buf, addr).await?)
  }

  pub async fn send_to_with_empty_meta<T>(&self, data: &T, addr: SocketAddr) -> Result<usize, Error>
  where
    T: Serialize,
  {
    let mut buf = vec![0u8; 2];
    self.coder.serialize_into(&mut buf, data)?;
    Ok(self.send_to_raw(&buf, addr).await?)
  }
}

pub struct Connection<Coder>
where
  Coder: Options + Copy,
{
  // TODO: use a flatten BtreeMap
  pending_works: Arc<Mutex<BTreeMap<SocketAddr, BTreeMap<u16, sync::oneshot::Sender<Vec<u8>>>>>>,
  counters: Arc<Mutex<BTreeMap<SocketAddr, atomic::AtomicU16>>>,
  inner: Arc<SecureConnection<Coder>>,
  listener: task::JoinHandle<()>,
  timeout: Duration,
  retry_limits: u32,
}

impl<Coder: 'static + Options + Copy + Send + Sync> Connection<Coder> {
  pub fn as_inner(&self) -> &SecureConnection<Coder> {
    &self.inner
  }

  pub fn new(
    sock: UdpSocket,
    coder: Coder,
    pub_keys: Arc<RwLock<HashMap<SocketAddr, PublicKey>>>,
    timeout: Duration,
    retry_limits: u32,
  ) -> (
    Self,
    sync::mpsc::Receiver<(Vec<u8>, SocketAddr)>,
    sync::mpsc::Receiver<(PublicKey, SocketAddr)>,
  ) {
    let pending_works = Arc::new(Mutex::new(BTreeMap::<
      SocketAddr,
      BTreeMap<u16, sync::oneshot::Sender<Vec<u8>>>,
    >::new()));
    let (connection, pub_key_receiver) = SecureConnection::new(sock, pub_keys, coder);
    let connection = Arc::new(connection);

    let (sender, receiver) = sync::mpsc::channel::<(Vec<u8>, SocketAddr)>(100);

    let listener = tokio::spawn({
      let connection = connection.clone();
      let pending_works = pending_works.clone();
      async move {
        let mut buf = vec![0; 65535];
        loop {
          let (data, addr) = match connection.recv_from_raw(&mut buf).await {
            Ok(r) => r,
            Err(_) => continue, // TODO: log error
          };
          let id = NetworkEndian::read_u16(&data[..]);
          let data = data[2..].to_vec();
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
        inner: connection,
        timeout,
        retry_limits,
        counters: Default::default(),
      },
      receiver,
      pub_key_receiver,
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

    self.inner.get_coder().serialize_into(&mut buf, req)?;

    let buf = self.inner.secure_serialize(&buf[..], addr)?;

    let mut retry_counter = self.retry_limits;

    loop {
      self.inner.send_to_insecurely(&buf, addr).await?;

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
          return Ok(self.inner.get_coder().deserialize::<Res>(&buf)?);
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

  pub fn release(&self, addr: SocketAddr) {
    self.inner.release(addr);
    self.counters.lock().remove(&addr);
    self.pending_works.lock().remove(&addr);
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
  #[error("error occurred during encryption")]
  EncryptionFailed,
  #[error("error occurred during decryption")]
  DecryptionFailed,
  #[error("public key for given destination not found")]
  NoDestKey,
  #[error("public key for given source not found")]
  NoSrcKey,
}
