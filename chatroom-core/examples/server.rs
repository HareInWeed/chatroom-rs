use std::{collections::HashMap, net::SocketAddr, result::Result, sync::Arc, time::Duration};

use tokio::{self, net::UdpSocket, task::JoinHandle};

use chatroom_core::{
  data::{default_coder, Command, ErrorCode, Notification, Response, ResponseData, User, UserInfo},
  utils::{default_timeout, Error},
};

use argon2;
use rand::Rng;

use parking_lot::{RwLock, RwLockUpgradableReadGuard, RwLockWriteGuard};

use bincode::Options;

use clap::Parser;

use byteorder::{ByteOrder, NetworkEndian};

use futures::future::join_all;

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
struct State<Coder: Options + Copy> {
  addr2user: RwHashMap<SocketAddr, String>,
  users: RwHashMap<String, User>,
  user_active_timers: RwHashMap<String, JoinHandle<()>>,
  coder: Coder,
  heartbeat_interval: Duration,
}

impl<Coder: Options + Copy> State<Coder> {
  fn new(coder: Coder, heartbeat_interval: Duration) -> Self {
    Self {
      addr2user: Default::default(),
      users: Default::default(),
      user_active_timers: Default::default(),
      heartbeat_interval,
      coder,
    }
  }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
  let args = Args::parse();

  let state = Arc::new(State::new(default_coder(), Duration::from_secs(60)));

  let sock = Arc::new(UdpSocket::bind(&args.addr).await?);
  println!("server running at {}", sock.local_addr()?);

  let mut buf = vec![0; 65535];

  loop {
    let (len, addr) = match sock.recv_from(&mut buf).await {
      Ok(req) => req,
      Err(err) => {
        eprintln!("{}", Error::from(err));
        continue;
      }
    };

    let buf = buf[..len].to_vec();
    let sock = sock.clone();
    let state = state.clone();
    tokio::spawn(async move {
      print!("[{}] ", addr);
      if let Err(err) = process(state, sock, buf, addr).await {
        println!("{:?}", err);
      }
    });
  }
}

async fn process<Coder: 'static + Options + Copy + Send + Sync>(
  state: Arc<State<Coder>>,
  sock: Arc<UdpSocket>,
  buf: Vec<u8>,
  addr: SocketAddr,
) -> Result<(), Error> {
  let id = NetworkEndian::read_u16(&buf[..]);
  let command = state.coder.deserialize::<Command>(&buf[2..])?;
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
            ip_address: addr,
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

        // update activity timer
        let mut user_active_timers = state.user_active_timers.write();
        let old_timer = user_active_timers.insert(username.clone(), {
          let state = state.clone();
          let sock = sock.clone();
          let username = username.clone();
          tokio::spawn(async move {
            tokio::time::sleep(state.heartbeat_interval).await;
            announce_offline(state, username, sock).await;
          })
        });
        let user_active_timers = RwLockWriteGuard::<_>::downgrade(user_active_timers);

        if let Some(old_timer) = old_timer {
          old_timer.abort();
        }

        // update map from addr to user
        let old_addr = users.get(&username).unwrap().ip_address;
        state.addr2user.write().remove(&old_addr);
        let users = if old_addr == addr {
          users
        } else {
          let mut users = RwLockUpgradableReadGuard::<_>::upgrade(users);
          users.get_mut(&username).unwrap().ip_address = addr;
          RwLockWriteGuard::<_>::downgrade_to_upgradable(users)
        };

        state.addr2user.write().insert(addr, username.clone());

        // broadcast online message
        {
          let user_info = UserInfo::new(state.users.read().get(&username).unwrap(), true); // TODO: log error
          let state = state.clone();
          let sock = sock.clone();
          tokio::spawn(async move {
            announce_online(state, user_info, sock).await // TODO log error
          });
        }

        // generate all user info
        let users_info = users
          .iter()
          .map(|(_, user)| UserInfo::new(user, user_active_timers.contains_key(&user.name)))
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
          .map(|(_, user)| UserInfo::new(user, user_active_timers.contains_key(&user.name)))
          .collect::<Vec<_>>(),
      });
    }),
    Command::Heartbeat => {
      if let Some(username) = state.addr2user.read().get(&addr).cloned() {
        if let Some(timer) = state.user_active_timers.write().get_mut(&username) {
          timer.abort();
          let state = state.clone();
          let sock = sock.clone();
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
          let timer = state.user_active_timers.write().remove(&username);
          if let Some(timer) = timer {
            timer.abort();
            let state = state.clone();
            let sock = sock.clone();
            tokio::spawn(async move {
              announce_offline(state, username, sock).await // TODO: log error
            });
            Ok(ResponseData::Success)
          } else {
            Err(ErrorCode::LoginRequired)
          }
        }
        None => Err(ErrorCode::LoginRequired),
      })
    }
    _ => Some(Err(ErrorCode::Unsupported)),
  };

  if let Some(response) = response {
    let mut buf = vec![0u8, 2];
    NetworkEndian::write_u16(&mut buf, id);
    state.coder.serialize_into(&mut buf, &response)?;
    default_timeout(sock.send_to(buf.as_slice(), addr)).await??;
  }

  Ok(())
}

async fn announce_online<Coder: Options + Copy>(
  state: Arc<State<Coder>>,
  user_info: UserInfo,
  sock: Arc<UdpSocket>,
) {
  let username = user_info.name.clone();
  let notification = Notification::Online(user_info);
  let mut buf = vec![0u8, 2];
  state.coder.serialize_into(&mut buf, &notification).unwrap(); // TODO: log error

  let futures = state
    .addr2user
    .read()
    .iter()
    .filter_map(|(&addr, name)| {
      if name != &username {
        Some(default_timeout(sock.send_to(buf.as_slice(), addr)))
      } else {
        None
      }
    })
    .collect::<Vec<_>>();

  join_all(futures).await;
}

async fn announce_offline<Coder: Options + Copy>(
  state: Arc<State<Coder>>,
  username: String,
  sock: Arc<UdpSocket>,
) {
  let notification = Notification::Offline(username);
  let mut buf = vec![0u8, 2];
  state.coder.serialize_into(&mut buf, &notification).unwrap(); // TODO: log error

  let futures = state
    .addr2user
    .read()
    .iter()
    .map(|(&addr, _)| default_timeout(sock.send_to(buf.as_slice(), addr)))
    .collect::<Vec<_>>();
  join_all(futures).await;
}
