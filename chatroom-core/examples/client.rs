use std::{
  collections::{BTreeMap, HashMap},
  io::{self, Write},
  net::{self, SocketAddr},
  result::Result,
  sync::Arc,
  time::Duration as StdDuration,
};

use bincode::Options;
use futures::future::join_all;
use parking_lot::{Mutex, RwLock};
use tokio::{net::UdpSocket, task::JoinHandle};

use clap::Parser;

use chatroom_core::{
  connection::Connection,
  data::{
    default_coder, Command, ErrorCode, Message, Notification, Response, ResponseData, UserInfo,
  },
  utils::Error,
};

use time::OffsetDateTime;

use sha2::{Digest, Sha256};

/// Chatroom client
#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
  /// specify socket address
  #[clap(short, long, default_value = "0.0.0.0:0")]
  addr: String,
}

type RwHashMap<K, V> = RwLock<HashMap<K, V>>;
type RwBTreeMap<K, V> = RwLock<BTreeMap<K, V>>;

#[derive(Debug, Clone)]
enum ChatEntry {
  Online,
  Offline,
  Message(String),
}

#[derive(Debug, Clone)]
struct OwnedChatEntry {
  user: String,
  entry: ChatEntry,
}

impl OwnedChatEntry {
  fn new(user: String, entry: ChatEntry) -> Self {
    Self { user, entry }
  }
}

#[derive(Debug, Clone)]
struct PersonalInfo {
  name: String,
  ip_address: SocketAddr,
}

#[derive(Debug)]
struct State {
  addr2user: RwHashMap<SocketAddr, String>,
  users: RwHashMap<String, UserInfo>,
  group_history: RwBTreeMap<OffsetDateTime, OwnedChatEntry>,
  ono2one_history: RwHashMap<String, BTreeMap<OffsetDateTime, ChatEntry>>,
  personal_info: Arc<Mutex<Option<PersonalInfo>>>,
  heartbeat_timer: Arc<Mutex<Option<JoinHandle<()>>>>,
  heartbeat_interval: StdDuration,
}

impl State {
  fn new(heartbeat_interval: StdDuration) -> Self {
    State {
      addr2user: Default::default(),
      users: Default::default(),
      group_history: Default::default(),
      ono2one_history: Default::default(),
      personal_info: Default::default(),
      heartbeat_timer: Default::default(),
      heartbeat_interval,
    }
  }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
  let args = Args::parse();

  let sock = UdpSocket::bind(args.addr).await?;

  println!("client started at {}", sock.local_addr()?);

  // server addr
  let server_addr = {
    let mut server_addr = String::new();
    loop {
      print!("[[client]] input server address: ");
      io::stdout().flush().map_err(Error::StdIO)?;

      server_addr.clear();
      io::stdin()
        .read_line(&mut server_addr)
        .map_err(Error::StdIO)?;

      if let Ok(addr) = server_addr.trim().parse::<net::SocketAddr>() {
        break addr;
      } else {
        eprintln!("[[client]] invalid server address {}", &server_addr);
      }
    }
  };

  let state = Arc::new(State::new(StdDuration::from_secs(30)));

  let coder = default_coder();

  let (connection, receiver) = Connection::new(sock, coder, StdDuration::from_secs(5), 5);
  let connection = Arc::new(connection);

  {
    let state = state.clone();
    let coder = coder.clone();
    let mut receiver = receiver;
    tokio::spawn(async move {
      loop {
        match receiver.recv().await {
          Some((buf, source)) => {
            if source == server_addr {
              // from server
              match coder.deserialize::<Notification>(&buf[..]) {
                Ok(Notification::Offline {
                  timestamp: time,
                  username: name,
                }) => {
                  let addr = match state.users.read().get(&name) {
                    Some(s) => s.ip_address,
                    None => continue,
                  };

                  if let None = state.addr2user.write().remove(&addr) {
                    continue;
                  }

                  println!("[{}: is offline]", name);
                  state.users.write().get_mut(&name).unwrap().is_online = false;
                  state
                    .group_history
                    .write()
                    .insert(time, OwnedChatEntry::new(name.clone(), ChatEntry::Offline));
                  state
                    .ono2one_history
                    .write()
                    .entry(name)
                    .or_default()
                    .insert(time, ChatEntry::Offline);
                }
                Ok(Notification::Online {
                  timestamp: time,
                  user_info: user,
                }) => {
                  let addr = user.ip_address;
                  let name = user.name.clone();

                  println!("[{}: is online]", name);

                  state.addr2user.write().insert(addr, name.clone());
                  state.users.write().insert(name.clone(), user);
                  state
                    .group_history
                    .write()
                    .insert(time, OwnedChatEntry::new(name.clone(), ChatEntry::Online));
                  state
                    .ono2one_history
                    .write()
                    .entry(name)
                    .or_default()
                    .insert(time, ChatEntry::Online);
                }
                Ok(_) => {
                  // log error
                }
                Err(_) => {
                  // log error
                  continue;
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
                      println!("[{}] {}", name, &msg);
                      state.group_history.write().insert(
                        timestamp,
                        OwnedChatEntry::new(name.clone(), ChatEntry::Message(msg)),
                      );
                    } else {
                      println!("[{}: to you] {}", name, &msg);
                      state
                        .ono2one_history
                        .write()
                        .entry(name.clone())
                        .or_default()
                        .insert(timestamp, ChatEntry::Message(msg));
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
            // TODO: log error
          }
        }
      }
    });
  }

  let mut input = String::new();
  loop {
    input.clear();
    io::stdin().read_line(&mut input).map_err(Error::StdIO)?;

    if let Some((command, args)) = input.as_str().trim_start().split_once(' ') {
      let mut args_iter = args.trim().split_whitespace();
      match command {
        "REGISTER" => {
          if let (Some(name), Some(pass), None) =
            (args_iter.next(), args_iter.next(), args_iter.next())
          {
            let mut hasher = Sha256::new();
            hasher.update(pass.trim_start());
            let password = hasher.finalize().into();
            match connection
              .request::<_, Response>(
                &Command::Register {
                  username: name.into(),
                  password,
                },
                server_addr,
              )
              .await?
            {
              Ok(ResponseData::Success) => {
                println!("[[server]] Succeeded, now you can login as \"{}\"", name);
              }
              Ok(response) => eprintln!("[[client]] unexpected response {:?}", response),
              Err(ErrorCode::UserExisted) => eprintln!("[[server]] username is occupied"),
              Err(error) => eprintln!("[[server]] operation failed: {:?}", error),
            }
          } else {
            eprintln!("[[client]] Invalid command");
          }
        }
        "LOGIN" => {
          if let (Some(name), Some(pass), None) =
            (args_iter.next(), args_iter.next(), args_iter.next())
          {
            let mut hasher = Sha256::new();
            hasher.update(pass.trim_start());
            let password = hasher.finalize().into();
            match connection
              .request::<_, Response>(
                &Command::Login {
                  username: name.into(),
                  password,
                },
                server_addr,
              )
              .await?
            {
              Ok(ResponseData::ChatroomStatus { users }) => {
                let timer = tokio::spawn({
                  let connection = connection.clone();
                  let mut interval = tokio::time::interval(state.heartbeat_interval);
                  async move {
                    loop {
                      interval.tick().await;
                      if let Err(_) = connection
                        .send_to_with_meta(&Command::Heartbeat, server_addr)
                        .await
                      {
                        // TODO: log error
                      }
                    }
                  }
                });

                *state.addr2user.write() = users
                  .iter()
                  .filter_map(|u| {
                    if u.is_online {
                      Some((u.ip_address.clone(), u.name.clone()))
                    } else {
                      None
                    }
                  })
                  .collect();
                *state.users.write() = users.into_iter().map(|u| (u.name.clone(), u)).collect();
                let my_addr = state
                  .users
                  .read()
                  .get(name)
                  .map(|u| u.ip_address.clone())
                  .unwrap(); // TODO: log error
                *state.personal_info.lock() = Some(PersonalInfo {
                  name: name.into(),
                  ip_address: my_addr,
                });
                *state.heartbeat_timer.lock() = Some(timer);
                println!("[[server]] You have logged in as \"{}\"", name);
              }
              Ok(response) => eprintln!("[[client]] unexpected response {:?}", response),
              Err(ErrorCode::InvalidUserOrPass) => {
                eprintln!("[[server]] username or password is incorrect")
              }
              Err(error) => eprintln!("[[server]] operation failed: {:?}", error),
            }
          } else {
            eprintln!("[[client]] Invalid command");
          }
        }
        "CHANGE_PASS" => {
          if let (Some(old), Some(new), None) =
            (args_iter.next(), args_iter.next(), args_iter.next())
          {
            let mut hasher = Sha256::new();
            hasher.update(old.trim_start());
            let old = hasher.finalize().into();

            let mut hasher = Sha256::new();
            hasher.update(new.trim_start());
            let new = hasher.finalize().into();

            match connection
              .request::<_, Response>(&Command::ChangePassword { old, new }, server_addr)
              .await?
            {
              Ok(ResponseData::Success) => println!("[[server]] Succeeded"),
              Ok(response) => eprintln!("[[client]] unexpected response {:?}", response),
              Err(ErrorCode::InvalidUserOrPass) => {
                eprintln!("[[server]] username or password is incorrect")
              }
              Err(error) => eprintln!("[[server]] operation failed: {:?}", error),
            }
          } else {
            eprintln!("[[client]] Invalid command");
          }
        }
        "SAY_TO" => {
          if let Some((username, msg)) = args.split_once(' ') {
            if let Some(UserInfo {
              name,
              ip_address,
              is_online,
            }) = state.users.read().get(username).cloned()
            {
              if is_online {
                let timestamp = OffsetDateTime::now_utc();
                connection
                  .send_to_with_meta(
                    &Message {
                      to_all: false,
                      timestamp,
                      msg: msg.into(),
                    },
                    ip_address,
                  )
                  .await?;
                state
                  .ono2one_history
                  .write()
                  .entry(name)
                  .or_default()
                  .insert(timestamp, ChatEntry::Message(msg.into()));
              } else {
                eprintln!("[[client]] User \"{}\" is offline", username);
              }
            } else {
              eprintln!("[[client]] User \"{}\" not found", username);
            }
          } else {
            eprintln!("[[client]] Invalid command");
          }
        }
        "SAY" => {
          let (my_name, my_addr) = match state
            .personal_info
            .lock()
            .as_ref()
            .map(|i| (i.name.clone(), i.ip_address.clone()))
          {
            Some(s) => s,
            None => {
              eprintln!("[[client]] You haven't logged in");
              continue;
            }
          };
          let msg = args;
          let timestamp = OffsetDateTime::now_utc();

          state.group_history.write().insert(
            timestamp,
            OwnedChatEntry::new(my_name, ChatEntry::Message(msg.into())),
          );

          let addrs = (state.users.read())
            .values()
            .filter_map(|user| {
              if user.ip_address != my_addr {
                Some(user.ip_address)
              } else {
                None
              }
            })
            .collect::<Vec<_>>();
          if let Err(_) = connection
            .send_to_multiple_with_meta(
              &Message {
                to_all: true,
                timestamp: OffsetDateTime::now_utc(),
                msg: msg.into(),
              },
              addrs.into_iter(),
            )
            .await
          {
            // TODO: log error
          }
        }
        _ => {
          eprintln!("[[client]] Invalid command");
        }
      }
    } else {
      // commands without argument
      match input.trim() {
        "STATUS" => {
          match connection
            .request::<_, Response>(&Command::GetChatroomStatus, server_addr)
            .await?
          {
            Ok(ResponseData::ChatroomStatus { users }) => {
              for user in users.iter() {
                if user.is_online {
                  println!("[[server]] \"{}\" is online", &user.name);
                }
              }
              for user in users.iter() {
                if !user.is_online {
                  println!("[[server]] \"{}\" is offline", &user.name);
                }
              }
              *state.addr2user.write() = users
                .iter()
                .filter_map(|u| {
                  if u.is_online {
                    Some((u.ip_address.clone(), u.name.clone()))
                  } else {
                    None
                  }
                })
                .collect();
              let my_addr = state
                .users
                .read()
                .get(&state.personal_info.lock().as_ref().unwrap().name)
                .map(|u| u.ip_address.clone())
                .unwrap(); // TODO: log error
              state.personal_info.lock().as_mut().unwrap().ip_address = my_addr;
              *state.users.write() = users.into_iter().map(|u| (u.name.clone(), u)).collect();
            }
            Ok(response) => eprintln!("[[client]] unexpected response {:?}", response),
            Err(ErrorCode::InvalidUserOrPass) => {
              eprintln!("[[server]] username or password is incorrect")
            }
            Err(error) => eprintln!("[[server]] operation failed: {:?}", error),
          }
        }
        "LOGOUT" => {
          // we don't care errors arise during logout
          let _ = connection
            .request::<_, Response>(&Command::Logout, server_addr)
            .await;
          *state.personal_info.lock() = None;
          if let Some(timer) = state.heartbeat_timer.lock().take() {
            timer.abort();
          }
          break;
        }
        _ => {
          eprintln!("[[client]] Invalid command");
        }
      }
    }
  }
  Ok(())
}
