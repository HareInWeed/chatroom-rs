use std::{collections::HashMap, net::SocketAddr, result::Result, sync::Arc, time::Duration};

use time::OffsetDateTime;
use tokio::{self, net::UdpSocket, task::JoinHandle};

use chatroom_core::{
  connection::SecureConnection,
  data::{
    default_coder, Command, ErrorCode, Notification, Response, ResponseData, User, UserInfo,
    UserOnlineInfo,
  },
  utils::Error,
};

use argon2;

use rand::Rng;

use parking_lot::{RwLock, RwLockUpgradableReadGuard, RwLockWriteGuard};

use bincode::Options;

use clap::Parser;

use byteorder::{ByteOrder, NetworkEndian};

use crypto_box::PublicKey;

/// Chatroom server
#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
  /// specify socket address of server
  #[clap(short, long, default_value = "0.0.0.0:0")]
  addr: String,
}

type RwHashMap<K, V> = RwLock<HashMap<K, V>>;

#[derive(Debug)]
struct State {
  addr2user: RwHashMap<SocketAddr, String>,
  users: RwHashMap<String, User>,
  user_active_timers: RwHashMap<String, JoinHandle<()>>,
  pub_keys: Arc<RwHashMap<SocketAddr, PublicKey>>,
  heartbeat_interval: Duration,
}

impl State {
  fn new(heartbeat_interval: Duration) -> Self {
    Self {
      addr2user: Default::default(),
      users: Default::default(),
      user_active_timers: Default::default(),
      pub_keys: Default::default(),
      heartbeat_interval,
    }
  }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
  let args = Args::parse();

  let state = Arc::new(State::new(Duration::from_secs(60)));

  let sock = UdpSocket::bind(&args.addr).await?;
  println!("server running at {}", sock.local_addr()?);

  let (connection, key_receiver) =
    SecureConnection::new(sock, state.pub_keys.clone(), default_coder());
  let connection = Arc::new(connection);

  tokio::spawn({
    let state = state.clone();
    let mut key_receiver = key_receiver;
    async move {
      loop {
        if let Some((key, addr)) = key_receiver.recv().await {
          if let Some(name) = state.addr2user.read().get(&addr) {
            if let Some(user) = state.users.write().get_mut(name) {
              if let Some(info) = user.online_info.as_mut() {
                info.pub_key = key.as_bytes().clone();
              }
            }
          }
        }
      }
    }
  });

  let mut buf = vec![0u8; 65535];

  loop {
    let (buf, addr) = match connection.recv_from_raw(&mut buf).await {
      Ok(req) => req,
      Err(err) => {
        eprintln!("{}", Error::from(err));
        continue;
      }
    };

    let connection = connection.clone();
    let state = state.clone();
    tokio::spawn(async move {
      print!("[{}] ", addr);
      if let Err(err) = process(state, connection, buf, addr).await {
        println!("{:?}", err);
      }
    });
  }
}

async fn process<Coder: 'static + Options + Copy + Send + Sync>(
  state: Arc<State>,
  connection: Arc<SecureConnection<Coder>>,
  buf: Vec<u8>,
  addr: SocketAddr,
) -> Result<(), Error> {
  let id = NetworkEndian::read_u16(&buf[..]);
  let command = connection.get_coder().deserialize::<Command>(&buf[2..])?;
  println!("{:?}", command);

  let response: Option<Response> = match command {
    Command::Register { username, password } => {
      Some(loop {
        let users = state.users.upgradable_read();
        if users.contains_key(&username) {
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
        break Ok(ResponseData::Success);
      })
    }
    Command::Login { username, password } => {
      let response: Response = loop {
        // check username and password
        let users = state.users.upgradable_read();
        let user = match users.get(&username) {
          Some(s) => s,
          None => break Err(ErrorCode::InvalidUserOrPass),
        };
        if !argon2::verify_encoded(&user.password_hash, &password).unwrap() {
          // TODO: log error
          break Err(ErrorCode::InvalidUserOrPass);
        }

        let pub_key = match state.pub_keys.read().get(&addr) {
          Some(pub_key) => pub_key.as_bytes().clone(),
          _ => break Err(ErrorCode::ConnectionNotSecure),
        };

        // update activity timer
        let old_timer = state.user_active_timers.write().insert(username.clone(), {
          let state = state.clone();
          let sock = connection.clone();
          let username = username.clone();
          tokio::spawn(async move {
            tokio::time::sleep(state.heartbeat_interval).await;
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
            if old_addr == addr {
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
          tokio::spawn(async move {
            announce_online(state, username, user_info, sock).await // TODO log error
          });
        }

        // generate all user info
        let users_info = users
          .iter()
          .map(|(_, user)| UserInfo::new(user))
          .collect::<Vec<_>>();

        break Ok(ResponseData::ChatroomStatus { users: users_info });
      };
      Some(response)
    }
    Command::ChangePassword { old, new } => {
      Some(loop {
        let addr2user = state.addr2user.read();
        let username = match addr2user.get(&addr) {
          Some(s) => s,
          None => break Err(ErrorCode::LoginRequired),
        };

        if !state.user_active_timers.read().contains_key(username) {
          break Err(ErrorCode::LoginRequired);
        }

        let users = state.users.upgradable_read();
        let user = users.get(username).unwrap(); // TODO: log error

        if !argon2::verify_encoded(&user.password_hash, &old).expect("failed to verify password") {
          break Err(ErrorCode::InvalidUserOrPass);
        }

        let mut salt = [0u8; 32];
        rand::thread_rng().fill(&mut salt);

        let password_hash = argon2::hash_encoded(&new, &salt, &argon2::Config::default()).unwrap(); // TODO: log error

        let mut users = RwLockUpgradableReadGuard::<_>::upgrade(users);
        users.get_mut(username).unwrap().password_hash = password_hash;

        break Ok(ResponseData::Success);
      })
    }
    Command::GetChatroomStatus => Some(loop {
      let addr2user = state.addr2user.read();
      let username = match addr2user.get(&addr) {
        Some(s) => s,
        None => break Err(ErrorCode::LoginRequired),
      };

      let user_active_timers = state.user_active_timers.read();

      if !user_active_timers.contains_key(username) {
        break Err(ErrorCode::LoginRequired);
      }

      break Ok(ResponseData::ChatroomStatus {
        users: state
          .users
          .read()
          .iter()
          .map(|(_, user)| UserInfo::new(user))
          .collect::<Vec<_>>(),
      });
    }),
    Command::Heartbeat => {
      if let Some(username) = state.addr2user.read().get(&addr).cloned() {
        if let Some(timer) = state.user_active_timers.write().get_mut(&username) {
          timer.abort();
          let state = state.clone();
          let sock = connection.clone();
          *timer = tokio::spawn(async move {
            tokio::time::sleep(state.heartbeat_interval).await;
            announce_offline(state, username, sock).await;
          });
        }
      };
      None
    }
    Command::Logout => {
      Some(match state.addr2user.write().remove(&addr) {
        Some(username) => {
          loop {
            let timer = state.user_active_timers.write().remove(&username);
            if let Some(timer) = timer {
              timer.abort();
            } else {
              break Err(ErrorCode::LoginRequired);
            }

            let user = match state.users.write().get_mut(&username) {
              Some(s) => s.online_info.take(),
              None => break Err(ErrorCode::LoginRequired),
            };

            if let Some(_) = user {
              let state = state.clone();
              let sock = connection.clone();
              tokio::spawn(async move {
                announce_offline(state, username, sock).await // TODO: log error
              });
              break Ok(ResponseData::Success);
            } else {
              break Err(ErrorCode::LoginRequired);
            }
          }
        }
        None => Err(ErrorCode::LoginRequired),
      })
    }
    _ => Some(Err(ErrorCode::Unsupported)),
  };

  if let Some(response) = response {
    connection.send_to_with_meta(&response, addr, id).await?;
  }

  Ok(())
}

async fn announce_online<Coder: 'static + Options + Copy + Send + Sync>(
  state: Arc<State>,
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
  state: Arc<State>,
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
