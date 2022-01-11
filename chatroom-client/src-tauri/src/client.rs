use std::{
  collections::{BTreeMap, HashMap},
  io::{self, Write},
  iter,
  net::{self, SocketAddr},
  result::Result,
  sync::Arc,
  time::Duration as StdDuration,
};

use bincode::Options;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::{net::UdpSocket, task::JoinHandle, time::timeout};

use chatroom_core::{
  connection::Connection,
  data::{
    Command, ErrorCode, Message, Notification, Response, ResponseData, UserInfo, UserOnlineInfo,
  },
  utils::Error,
};

use time::OffsetDateTime;

use sha2::{Digest, Sha256};

use crypto_box::PublicKey;

use tauri::{AppHandle, Manager};

type RwHashMap<K, V> = RwLock<HashMap<K, V>>;
type RwBTreeMap<K, V> = RwLock<BTreeMap<K, V>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChatEntry {
  Online,
  Offline,
  Message(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedChatEntry {
  user: String,
  entry: ChatEntry,
}

impl OwnedChatEntry {
  fn new(user: String, entry: ChatEntry) -> Self {
    Self { user, entry }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalInfo {
  name: String,
  ip_address: SocketAddr,
}

#[derive(Debug)]
pub struct ClientState {
  pub addr2user: RwHashMap<SocketAddr, String>,
  pub users: RwHashMap<String, UserInfo>,
  pub pub_keys: Arc<RwHashMap<SocketAddr, PublicKey>>,
  pub group_history: RwBTreeMap<OffsetDateTime, OwnedChatEntry>,
  pub ono2one_history: RwHashMap<String, BTreeMap<OffsetDateTime, OwnedChatEntry>>,
  pub personal_info: Arc<Mutex<Option<PersonalInfo>>>,
}

impl ClientState {
  fn new(heartbeat_interval: StdDuration) -> Self {
    ClientState {
      addr2user: Default::default(),
      users: Default::default(),
      pub_keys: Default::default(),
      group_history: Default::default(),
      ono2one_history: Default::default(),
      personal_info: Default::default(),
    }
  }
}

pub struct Client<Coder>
where
  Coder: 'static + Options + Copy + Sync + Send,
{
  pub client_addr: SocketAddr,
  pub server_addr: SocketAddr,
  state: Arc<ClientState>,
  connection: Arc<Connection<Coder>>,
  app_handle: AppHandle,
  net_receiver: JoinHandle<()>,
  heartbeat_timer: Arc<Mutex<Option<JoinHandle<()>>>>,
  heartbeat_interval: StdDuration,
}

impl<Coder> Client<Coder>
where
  Coder: 'static + Options + Copy + Sync + Send,
{
  pub async fn new(
    client_addr: SocketAddr,
    server_addr: SocketAddr,
    app_handle: AppHandle,
    coder: Coder,
    heartbeat_interval: StdDuration,
    request_timeout: StdDuration,
    retry_limits: u32,
  ) -> Result<Self, Error> {
    let sock = UdpSocket::bind(client_addr).await?;

    let state = Arc::new(ClientState::new(heartbeat_interval));

    let (connection, receiver, _) = Connection::new(
      sock,
      coder,
      state.pub_keys.clone(),
      request_timeout,
      retry_limits,
    );
    let connection = Arc::new(connection);

    timeout(
      request_timeout,
      connection.as_inner().exchange_key_with(server_addr),
    )
    .await??;

    let net_receiver = tokio::spawn({
      let state = state.clone();
      let coder = coder.clone();
      let connection = connection.clone();
      let app_handle = app_handle.clone();
      let mut receiver = receiver;
      async move {
        loop {
          match receiver.recv().await {
            Some((buf, source)) => {
              if source == server_addr {
                // from server
                match coder.deserialize::<Notification>(&buf[..]) {
                  Ok(Notification::Online {
                    timestamp: time,
                    name,
                    info,
                  }) => {
                    state
                      .addr2user
                      .write()
                      .insert(info.ip_address, name.clone());
                    connection
                      .as_inner()
                      .update_pub_keys(iter::once((info.pub_key.clone().into(), info.ip_address)));
                    // TODO: well, this won't handle new registered user really well,
                    // if future online unrelated info are included in user info
                    state
                      .users
                      .write()
                      .entry(name.clone())
                      .or_insert_with(|| UserInfo {
                        name: name.clone(),
                        online_info: None,
                      })
                      .online_info = Some(info);
                    state
                      .group_history
                      .write()
                      .insert(time, OwnedChatEntry::new(name.clone(), ChatEntry::Online));
                    state
                      .ono2one_history
                      .write()
                      .entry(name.clone())
                      .or_default()
                      .insert(time, OwnedChatEntry::new(name.clone(), ChatEntry::Online));

                    let _ = app_handle.emit_all("online", name);
                    let _ = app_handle.emit_all("new-msg", None::<String>);
                  }
                  Ok(Notification::Offline {
                    timestamp: time,
                    name,
                  }) => {
                    let online_info = match state.users.write().get_mut(&name) {
                      Some(user) => user.online_info.take(),
                      _ => continue,
                    };

                    let online_info = match online_info {
                      Some(s) => s,
                      None => continue,
                    };

                    connection.as_inner().release(online_info.ip_address);

                    if let None = state.addr2user.write().remove(&online_info.ip_address) {
                      continue;
                    }

                    state
                      .group_history
                      .write()
                      .insert(time, OwnedChatEntry::new(name.clone(), ChatEntry::Offline));

                    state
                      .ono2one_history
                      .write()
                      .entry(name.clone())
                      .or_default()
                      .insert(time, OwnedChatEntry::new(name.clone(), ChatEntry::Offline));

                    let _ = app_handle.emit_all("offline", name);
                    let _ = app_handle.emit_all("new-msg", None::<String>);
                  }
                  _ => {
                    // log error
                  }
                };
              } else {
                // from user
                match coder.deserialize::<Message>(&buf[..]) {
                  Ok(Message {
                    to_all,
                    msg,
                    timestamp,
                  }) => {
                    let addr2uesr = state.addr2user.read();

                    if let Some(name) = addr2uesr.get(&source) {
                      if to_all {
                        state.group_history.write().insert(
                          timestamp,
                          OwnedChatEntry::new(name.clone(), ChatEntry::Message(msg)),
                        );
                        let _ = app_handle.emit_all("new-msg", None::<String>);
                      } else {
                        state
                          .ono2one_history
                          .write()
                          .entry(name.clone())
                          .or_default()
                          .insert(
                            timestamp,
                            OwnedChatEntry::new(name.clone(), ChatEntry::Message(msg)),
                          );
                        let _ = app_handle.emit_all("new-msg", Some(&name));
                      }
                    }
                  }
                  Err(_) => {
                    // log error
                    continue;
                  }
                }
              }
            }
            None => {
              let _ = app_handle.emit_all("connection-lost", ());
            }
          }
        }
      }
    });

    Ok(Self {
      client_addr,
      server_addr,
      state,
      connection,
      app_handle,
      net_receiver,
      heartbeat_timer: Default::default(),
      heartbeat_interval,
    })
  }

  pub fn get_state(&self) -> Arc<ClientState> {
    self.state.clone()
  }

  pub async fn register(&self, name: String, pass: &str) -> Result<(), Error> {
    let mut hasher = Sha256::new();
    hasher.update(pass.trim_start());
    let password = hasher.finalize().into();
    match self
      .connection
      .request::<_, Response>(
        &Command::Register {
          username: name,
          password,
        },
        self.server_addr,
      )
      .await?
    {
      Ok(ResponseData::Success) => Ok(()),
      Err(ErrorCode::UserExisted) => Err(ErrorCode::UserExisted.into()),
      _ => Err(Error::UnsupportedResponse),
    }
  }

  pub async fn login(&self, name: String, pass: &str) -> Result<(), Error> {
    let mut hasher = Sha256::new();
    hasher.update(pass.trim_start());
    let password = hasher.finalize().into();
    match self
      .connection
      .request::<_, Response>(
        &Command::Login {
          username: name.clone(),
          password,
        },
        self.server_addr,
      )
      .await?
    {
      Ok(ResponseData::ChatroomStatus { users }) => {
        let timer = tokio::spawn({
          let connection = self.connection.clone();
          let server_addr = self.server_addr;
          let mut interval = tokio::time::interval(self.heartbeat_interval);
          async move {
            loop {
              interval.tick().await;
              if let Err(_) = connection
                .as_inner()
                .send_to_with_empty_meta(&Command::Heartbeat, server_addr)
                .await
              {
                // TODO: log error
              }
            }
          }
        });
        let old_timer = self.heartbeat_timer.lock().replace(timer);
        if let Some(old_timer) = old_timer {
          old_timer.abort();
        }

        *self.state.addr2user.write() = users
          .iter()
          .filter_map(|u| {
            if let UserInfo {
              online_info: Some(UserOnlineInfo { ip_address, .. }),
              name,
              ..
            } = u
            {
              Some((ip_address.clone(), name.clone()))
            } else {
              None
            }
          })
          .collect();
        self
          .connection
          .as_inner()
          .update_pub_keys(users.iter().filter_map(|u| {
            if let UserInfo {
              online_info:
                Some(UserOnlineInfo {
                  ip_address,
                  pub_key,
                  ..
                }),
              ..
            } = u
            {
              Some((pub_key.clone().into(), ip_address.clone()))
            } else {
              None
            }
          }));
        *self.state.users.write() = users.into_iter().map(|u| (u.name.clone(), u)).collect();

        let my_addr = {
          let users = self.state.users.read();
          // TODO: log error
          users
            .get(&name)
            .unwrap()
            .online_info
            .as_ref()
            .unwrap()
            .ip_address
        };

        *self.state.personal_info.lock() = Some(PersonalInfo {
          name: name.into(),
          ip_address: my_addr,
        });
        Ok(())
      }
      Err(ErrorCode::InvalidUserOrPass) => Err(ErrorCode::InvalidUserOrPass.into()),
      _ => Err(Error::UnsupportedResponse),
    }
  }

  pub async fn change_password(&self, old: &str, new: &str) -> Result<(), Error> {
    let mut hasher = Sha256::new();
    hasher.update(old.trim_start());
    let old = hasher.finalize().into();

    let mut hasher = Sha256::new();
    hasher.update(new.trim_start());
    let new = hasher.finalize().into();

    match self
      .connection
      .request::<_, Response>(&Command::ChangePassword { old, new }, self.server_addr)
      .await?
    {
      Ok(ResponseData::Success) => Ok(()),
      Err(ErrorCode::InvalidUserOrPass) => Err(ErrorCode::InvalidUserOrPass.into()),
      Err(ErrorCode::LoginRequired) => {
        let _ = self.app_handle.emit_all("not-login", ());
        Err(ErrorCode::LoginRequired.into())
      }
      _ => Err(Error::UnsupportedResponse),
    }
  }

  pub async fn say(&self, msg: String, username: Option<String>) -> Result<(), Error> {
    let (my_name, my_addr) = match self
      .state
      .personal_info
      .lock()
      .as_ref()
      .map(|i| (i.name.clone(), i.ip_address.clone()))
    {
      Some(s) => s,
      None => {
        let _ = self.app_handle.emit_all("not-login", ());
        return Err(ErrorCode::LoginRequired.into());
      }
    };
    if let Some(username) = username {
      // personal chat
      let user_info = self.state.users.read().get(&username).cloned();
      if let Some(UserInfo { name, online_info }) = user_info {
        if let Some(UserOnlineInfo { ip_address, .. }) = online_info {
          let timestamp = OffsetDateTime::now_utc();
          self
            .connection
            .as_inner()
            .send_to_with_empty_meta(
              &Message {
                to_all: false,
                timestamp,
                msg: msg.clone(),
              },
              ip_address,
            )
            .await?;
          self
            .state
            .ono2one_history
            .write()
            .entry(name)
            .or_default()
            .insert(
              timestamp,
              OwnedChatEntry::new(my_name, ChatEntry::Message(msg.clone())),
            );
          Ok(())
        } else {
          Err(ErrorCode::UserOffline.into())
        }
      } else {
        Err(ErrorCode::UserNotExisted.into())
      }
    } else {
      // public chat
      let timestamp = OffsetDateTime::now_utc();

      self.state.group_history.write().insert(
        timestamp,
        OwnedChatEntry::new(my_name, ChatEntry::Message(msg.clone())),
      );

      let addrs = (self.state.users.read())
        .values()
        .filter_map(|u| {
          if let Some(UserOnlineInfo { ip_address, .. }) = u.online_info {
            if my_addr != ip_address {
              Some(ip_address)
            } else {
              None
            }
          } else {
            None
          }
        })
        .collect::<Vec<_>>();
      if let Err(_) = self
        .connection
        .as_inner()
        .send_to_multiple_with_empty_meta(
          &Message {
            to_all: true,
            timestamp: OffsetDateTime::now_utc(),
            msg,
          },
          addrs.into_iter(),
        )
        .await
      {
        // TODO: log error
      }
      Ok(())
    }
  }

  pub async fn fetch_chatroom_status(&self) -> Result<(), Error> {
    match self
      .connection
      .request::<_, Response>(&Command::GetChatroomStatus, self.server_addr)
      .await?
    {
      Ok(ResponseData::ChatroomStatus { users }) => {
        *self.state.addr2user.write() = users
          .iter()
          .filter_map(|u| {
            if let Some(UserOnlineInfo { ip_address, .. }) = u.online_info {
              Some((ip_address, u.name.clone()))
            } else {
              None
            }
          })
          .collect();
        self
          .connection
          .as_inner()
          .update_pub_keys(users.iter().filter_map(|u| {
            if let UserInfo {
              online_info:
                Some(UserOnlineInfo {
                  ip_address,
                  pub_key,
                  ..
                }),
              ..
            } = u
            {
              Some((pub_key.clone().into(), ip_address.clone()))
            } else {
              None
            }
          }));
        let my_addr = (self.state.users.read())
          .get(&self.state.personal_info.lock().as_ref().unwrap().name)
          .map(|u| u.online_info.as_ref().unwrap().ip_address) // TODO: log error
          .unwrap(); // TODO: log error
        self.state.personal_info.lock().as_mut().unwrap().ip_address = my_addr;
        *self.state.users.write() = users.into_iter().map(|u| (u.name.clone(), u)).collect();
        Ok(())
      }
      Err(ErrorCode::LoginRequired) => {
        let _ = self.app_handle.emit_all("not-login", ());
        Err(ErrorCode::LoginRequired.into())
      }
      _ => Err(Error::UnsupportedResponse),
    }
  }

  pub async fn logout(self) -> Result<Option<Self>, Error> {
    let _ = self
      .connection
      .request::<_, Response>(&Command::Logout, self.server_addr)
      .await;
    *self.state.personal_info.lock() = None;
    if let Some(timer) = { self.heartbeat_timer.lock().take() } {
      timer.abort();
    };
    Ok(None)
  }
}

impl<Coder> Drop for Client<Coder>
where
  Coder: 'static + Options + Copy + Sync + Send,
{
  fn drop(&mut self) {
    self.net_receiver.abort();
    if let Some(timer) = { self.heartbeat_timer.lock().take() } {
      timer.abort();
    }
  }
}
