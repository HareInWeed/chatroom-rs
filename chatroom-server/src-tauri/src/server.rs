use std::{collections::HashMap, iter, net::SocketAddr, result::Result, sync::Arc, time::Duration};

use time::OffsetDateTime;
use tokio::{self, net::UdpSocket, task::JoinHandle};

use chatroom_core::{
  connection::SecureConnection,
  data::{
    Command, ErrorCode, Notification, Response, ResponseData, User, UserEssential, UserInfo,
    UserOnlineInfo,
  },
  utils::Error,
};

use argon2;

use rand::Rng;

use parking_lot::{RwLock, RwLockUpgradableReadGuard, RwLockWriteGuard};

use bincode::Options;

use byteorder::{ByteOrder, NetworkEndian};

use crypto_box::PublicKey;

use tauri::{AppHandle, Manager};

use time;

use tracing::{error, info, info_span};

type RwHashMap<K, V> = RwLock<HashMap<K, V>>;

#[derive(Debug)]
pub struct ServerState {
  pub addr2user: RwHashMap<SocketAddr, String>,
  pub users: RwHashMap<String, User>,
  pub user_active_timers: RwHashMap<String, JoinHandle<()>>,
  pub pub_keys: Arc<RwHashMap<SocketAddr, PublicKey>>,
  pub heartbeat_interval: Duration,
}

impl ServerState {
  pub fn new(heartbeat_interval: Duration) -> Self {
    Self::from_user_essentials(heartbeat_interval, iter::empty())
  }

  pub fn from_user_essentials<I>(heartbeat_interval: Duration, iter: I) -> Self
  where
    I: Iterator<Item = (String, UserEssential)>,
  {
    let users: HashMap<String, User> = iter.map(|(n, d)| (n.clone(), (n, d).into())).collect();
    let users = RwLock::new(users);
    Self {
      addr2user: Default::default(),
      users,
      user_active_timers: Default::default(),
      pub_keys: Default::default(),
      heartbeat_interval,
    }
  }

  pub fn get_user_essentials(&self) -> HashMap<String, UserEssential> {
    self
      .users
      .read()
      .iter()
      .map(|(n, d)| (n.clone(), d.into()))
      .collect()
  }
}

pub struct Server<Coder>
where
  Coder: Options + Copy,
{
  state: Arc<ServerState>,
  connection: Arc<SecureConnection<Coder>>,

  key_receiver: Option<JoinHandle<()>>,
  req_receiver: Option<JoinHandle<()>>,
}

impl<Coder> Server<Coder>
where
  Coder: 'static + Options + Copy + Send + Sync,
{
  pub async fn new<I>(
    coder: Coder,
    users: I,
    app_handle: AppHandle,
    heartbeat_interval: Duration,
    server_addr: &str,
  ) -> Result<Server<Coder>, Error>
  where
    I: Iterator<Item = (String, UserEssential)>,
  {
    let state = Arc::new(ServerState::from_user_essentials(heartbeat_interval, users));
    let sock = UdpSocket::bind(server_addr).await?;

    info!(
      source = "server",
      "server started at {}.",
      sock.local_addr()?
    );

    let (connection, key_receiver) = SecureConnection::new(sock, state.pub_keys.clone(), coder);
    let connection = Arc::new(connection);

    let key_receiver = tokio::spawn({
      let state = state.clone();
      let mut key_receiver = key_receiver;
      let app_handle = app_handle.clone();
      async move {
        loop {
          if let Some((key, addr)) = key_receiver.recv().await {
            if let Some(name) = state.addr2user.read().get(&addr) {
              if let Some(user) = state.users.write().get_mut(name) {
                if let Some(info) = user.online_info.as_mut() {
                  let _ = app_handle.emit_all("user-info-updated", ());
                  info.pub_key = key.as_bytes().clone();
                }
              }
            }
          }
        }
      }
    });

    let req_receiver = tokio::spawn({
      let connection = connection.clone();
      let state = state.clone();
      async move {
        let mut buf = vec![0u8; 65535];

        loop {
          let (buf, addr) = match connection.recv_from_raw(&mut buf).await {
            Ok(req) => req,
            Err(err) => {
              error!(
                source = "internal",
                "error occurred during receiving request: {}.", err
              );
              continue;
            }
          };

          let connection = connection.clone();
          let state = state.clone();
          tokio::spawn({
            let app_handle = app_handle.clone();
            async move {
              if let Err(err) = process(state, connection, app_handle.clone(), buf, addr).await {
                error!(
                  source = "internal",
                  "error occurred during processing request: {}.", err
                );
              }
            }
          });
        }
      }
    });

    Ok(Self {
      state,
      connection,
      key_receiver: Some(key_receiver),
      req_receiver: Some(req_receiver),
    })
  }

  pub fn get_state(&self) -> Arc<ServerState> {
    self.state.clone()
  }
}

impl<Coder> Drop for Server<Coder>
where
  Coder: Options + Copy,
{
  fn drop(&mut self) {
    if let Some(handle) = self.key_receiver.take() {
      handle.abort();
    }
    if let Some(handle) = self.req_receiver.take() {
      handle.abort();
    }
    for (_, timer) in self.state.user_active_timers.write().iter() {
      timer.abort();
    }
  }
}

async fn process<Coder: 'static + Options + Copy + Send + Sync>(
  state: Arc<ServerState>,
  connection: Arc<SecureConnection<Coder>>,
  app_handle: AppHandle,
  buf: Vec<u8>,
  addr: SocketAddr,
) -> Result<(), Error> {
  let id = NetworkEndian::read_u16(&buf[..]);
  let command = connection.get_coder().deserialize::<Command>(&buf[2..])?;

  let response: Option<Response> = match command {
    Command::Register { username, password } => {
      let _span =
        info_span!("REGISTER", %addr, username = username.as_str(), password = "...").entered();
      info!("new request.");
      Some(loop {
        let users = state.users.upgradable_read();
        if users.contains_key(&username) {
          error!(source = "server", "user \"{}\" is occupied.", &username);
          break Err(ErrorCode::UserExisted);
        }

        let mut salt = [0u8; 32];
        rand::thread_rng().fill(&mut salt);

        let password_hash =
          argon2::hash_encoded(&password, &salt, &argon2::Config::default()).unwrap(); // TODO: log error

        let mut users = RwLockUpgradableReadGuard::<_>::upgrade(users);
        users.insert(
          username.clone(),
          User {
            name: username.clone(),
            password_hash,
            online_info: None,
          },
        );

        let _ = app_handle.emit_all("user-info-updated", ());
        info!(
          source = "server",
          "user \"{}\" registered successfully.", &username
        );
        break Ok(ResponseData::Success);
      })
    }
    Command::Login { username, password } => {
      let _span =
        info_span!("LOGIN", %addr, username = username.as_str(),password = "...").entered();
      info!("new request.");
      let response: Response = loop {
        // check username and password
        let users = state.users.upgradable_read();
        let user = match users.get(&username) {
          Some(s) => s,
          None => {
            error!("user \"{}\" does not exist", &username);
            break Err(ErrorCode::InvalidUserOrPass);
          }
        };
        if !argon2::verify_encoded(&user.password_hash, &password).unwrap() {
          error!(
            source = "server",
            "password for user \"{}\" is incorrect.", &username
          );
          break Err(ErrorCode::InvalidUserOrPass);
        }

        let pub_key = match state.pub_keys.read().get(&addr) {
          Some(pub_key) => pub_key.as_bytes().clone(),
          _ => {
            error!(
              source = "server",
              "failed to find public key of user \"{}\".", &username
            );
            break Err(ErrorCode::ConnectionNotSecure);
          }
        };

        // update activity timer
        let old_timer = state.user_active_timers.write().insert(username.clone(), {
          let state = state.clone();
          let sock = connection.clone();
          let username = username.clone();
          let app_handle = app_handle.clone();
          tokio::spawn(async move {
            tokio::time::sleep(state.heartbeat_interval).await;
            state.user_active_timers.write().remove(&username);
            let online_info = match state.users.write().get_mut(&username) {
              Some(user) => user.online_info.take(),
              None => None,
            };
            if let Some(UserOnlineInfo { ip_address, .. }) = online_info {
              state.addr2user.write().remove(&ip_address);
            }
            let _ = app_handle.emit_all("user-info-updated", ());
            info!(
              source = "server",
              "heartbeat signal of user \"{}\" is lost.", &username
            );
            announce_offline(state, username, sock).await;
          })
        });

        if let Some(old_timer) = old_timer {
          old_timer.abort();
        }

        // update user and map from addr to user
        let mut users = RwLockUpgradableReadGuard::<_>::upgrade(users);
        let user_info = {
          let user = users.get_mut(&username).unwrap();
          let old_online_info = user.online_info.take();
          if let Some(UserOnlineInfo {
            ip_address: old_addr,
            ..
          }) = old_online_info
          {
            if old_addr != addr {
              state.addr2user.write().remove(&old_addr);
            }
          }

          let info = UserOnlineInfo {
            ip_address: addr,
            pub_key,
          };
          user.online_info = Some(info.clone());
          info
        };
        let users = RwLockWriteGuard::<_>::downgrade_to_upgradable(users);

        state.addr2user.write().insert(addr, username.clone());

        // broadcast online message
        {
          let state = state.clone();
          let sock = connection.clone();
          let username = username.clone();
          tokio::spawn(async move {
            announce_online(state, username, user_info, sock).await // TODO log error
          });
        }

        // generate all user info
        let users_info = users
          .iter()
          .map(|(_, user)| UserInfo::new(user))
          .collect::<Vec<_>>();

        let _ = app_handle.emit_all("user-info-updated", ());
        info!(
          source = "server",
          "user \"{}\" logged in successfully.", &username
        );

        break Ok(ResponseData::ChatroomStatus { users: users_info });
      };
      Some(response)
    }
    Command::ChangePassword { old, new } => {
      let _span = info_span!("CHANGE_PASSWORD", %addr, old_pass="...", new_pass="...").entered();
      info!("new request.");
      Some(loop {
        let addr2user = state.addr2user.read();
        let username = match addr2user.get(&addr) {
          Some(s) => s,
          None => {
            error!(source = "server", "no online user binds to the address.");
            break Err(ErrorCode::LoginRequired);
          }
        };

        if !state.user_active_timers.read().contains_key(username) {
          error!(source = "server", "user \"{}\" is not online.", &username);
          break Err(ErrorCode::LoginRequired);
        }

        let users = state.users.upgradable_read();
        let user = users.get(username).unwrap(); // TODO: log error

        if !argon2::verify_encoded(&user.password_hash, &old).unwrap() {
          error!(
            source = "server",
            "old password for user \"{}\" is incorrect.", &username
          );
          break Err(ErrorCode::InvalidUserOrPass);
        }

        let mut salt = [0u8; 32];
        rand::thread_rng().fill(&mut salt);

        let password_hash = argon2::hash_encoded(&new, &salt, &argon2::Config::default()).unwrap(); // TODO: log error

        let mut users = RwLockUpgradableReadGuard::<_>::upgrade(users);

        let _ = app_handle.emit_all("user-info-updated", ());
        users.get_mut(username).unwrap().password_hash = password_hash;
        info!(
          source = "server",
          "user \"{}\" changed password successfully.", &username
        );

        break Ok(ResponseData::Success);
      })
    }
    Command::GetChatroomStatus => Some(loop {
      let _span = info_span!("GET_CHATROOM_STATUS", %addr).entered();
      info!("new request.");

      let addr2user = state.addr2user.read();
      let username = match addr2user.get(&addr) {
        Some(s) => s,
        None => {
          error!(source = "server", "no online user binds to the address.");
          break Err(ErrorCode::LoginRequired);
        }
      };

      let user_active_timers = state.user_active_timers.read();

      if !user_active_timers.contains_key(username) {
        error!(source = "server", "user \"{}\" is not online.", &username);
        break Err(ErrorCode::LoginRequired);
      }

      let respond = Ok(ResponseData::ChatroomStatus {
        users: state
          .users
          .read()
          .iter()
          .map(|(_, user)| UserInfo::new(user))
          .collect::<Vec<_>>(),
      });

      info!(
        source = "server",
        "user \"{}\" queried status successfully.", &username
      );

      break respond;
    }),
    Command::Heartbeat => {
      let _span = info_span!("HEARTBEAT", %addr).entered();
      info!("new request.");
      if let Some(username) = state.addr2user.read().get(&addr).cloned() {
        if let Some(timer) = state.user_active_timers.write().get_mut(&username) {
          timer.abort();
          let state = state.clone();
          let sock = connection.clone();
          *timer = tokio::spawn({
            let username = username.clone();
            let app_handle = app_handle.clone();
            async move {
              tokio::time::sleep(state.heartbeat_interval).await;
              state.user_active_timers.write().remove(&username);
              let online_info = match state.users.write().get_mut(&username) {
                Some(user) => user.online_info.take(),
                None => None,
              };
              if let Some(UserOnlineInfo { ip_address, .. }) = online_info {
                state.addr2user.write().remove(&ip_address);
              }
              let _ = app_handle.emit_all("user-info-updated", ());
              info!(
                source = "server",
                "heartbeat signal of user \"{}\" is lost.", &username
              );
              announce_offline(state, username, sock).await;
            }
          });
          info!(
            source = "server",
            "activity timer for user \"{}\" is updated.", &username
          );
        } else {
          error!(source = "server", "user \"{}\" is not online.", &username);
        }
      } else {
        error!(source = "server", "no online user binds to the address.");
      };
      None
    }
    Command::Logout => {
      let _span = info_span!("LOGOUT", %addr).entered();
      info!("new request.");
      Some(match state.addr2user.write().remove(&addr) {
        Some(username) => {
          loop {
            let timer = state.user_active_timers.write().remove(&username);
            if let Some(timer) = timer {
              timer.abort();
            } else {
              error!(source = "internal", "user \"{}\" is not online.", &username);
              break Err(ErrorCode::LoginRequired);
            }

            let user = match state.users.write().get_mut(&username) {
              Some(s) => s.online_info.take(),
              None => {
                error!(
                  source = "internal",
                  "user \"{}\" does not existed.", &username
                );
                break Err(ErrorCode::LoginRequired);
              }
            };

            if let Some(_) = user {
              let state = state.clone();
              let sock = connection.clone();
              tokio::spawn({
                let username = username.clone();
                async move {
                  announce_offline(state, username, sock).await // TODO: log error
                }
              });

              let _ = app_handle.emit_all("user-info-updated", ());
              info!(
                source = "server",
                "user \"{}\" logout successfully.", &username
              );
              break Ok(ResponseData::Success);
            } else {
              error!(
                source = "internal",
                "online info of user \"{}\" is empty.", &username
              );
              break Err(ErrorCode::LoginRequired);
            }
          }
        }
        None => {
          error!(source = "server", "no online user binds to the address.");
          Err(ErrorCode::LoginRequired)
        }
      })
    }
    cmd => {
      error!(source = "internal", "Unsupported Message: \"{:?}\".", &cmd);
      Some(Err(ErrorCode::Unsupported))
    }
  };

  if let Some(response) = response {
    connection.send_to_with_meta(&response, addr, id).await?;
  }

  Ok(())
}

async fn announce_online<Coder: 'static + Options + Copy + Send + Sync>(
  state: Arc<ServerState>,
  name: String,
  info: UserOnlineInfo,
  connection: Arc<SecureConnection<Coder>>,
) {
  let addrs = state
    .addr2user
    .read()
    .iter()
    .filter_map(|(&addr, n)| if n != &name { Some(addr) } else { None })
    .collect::<Vec<_>>();

  let notification = Notification::Online {
    timestamp: OffsetDateTime::now_utc(),
    name,
    info,
  };

  if let Err(_) = connection
    .send_to_multiple_with_empty_meta(&notification, addrs.into_iter())
    .await
  { // TODO: log error
  }
}

async fn announce_offline<Coder: 'static + Options + Copy + Send + Sync>(
  state: Arc<ServerState>,
  name: String,
  connection: Arc<SecureConnection<Coder>>,
) {
  let addrs = state
    .addr2user
    .read()
    .iter()
    .filter_map(|(&addr, n)| if n != &name { Some(addr) } else { None })
    .collect::<Vec<_>>();

  let notification = Notification::Offline {
    timestamp: OffsetDateTime::now_utc(),
    name,
  };

  if let Err(_) = connection
    .send_to_multiple_with_empty_meta(&notification, addrs.into_iter())
    .await
  { // TODO: log error
  }
}
