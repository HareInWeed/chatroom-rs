use std::{
  collections::HashMap,
  io::{self, Write},
  net,
  result::Result,
  sync::Arc,
  time::Duration as StdDuration,
};

use tokio::net::UdpSocket;

use clap::Parser;

use chatroom_core::{
  connection::Connection,
  data::{default_coder, Command, Response},
  utils::Error,
};

use parking_lot::RwLock;

use sha2::{Digest, Sha256};

use crypto_box::PublicKey;

/// Chatroom client
#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
  /// specify socket address
  #[clap(short, long, default_value = "0.0.0.0:0")]
  addr: String,
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
      print!("input server address: ");
      io::stdout().flush().map_err(Error::StdIO)?;

      server_addr.clear();
      io::stdin()
        .read_line(&mut server_addr)
        .map_err(Error::StdIO)?;

      if let Ok(addr) = server_addr.trim().parse::<net::SocketAddr>() {
        break addr;
      } else {
        eprintln!("invalid server address {}", &server_addr);
        io::stderr().flush().map_err(Error::StdIO)?;
      }
    }
  };

  let pub_keys: Arc<RwLock<HashMap<net::SocketAddr, PublicKey>>> = Default::default();

  let (connection, _, _) = Connection::new(
    sock,
    default_coder(),
    pub_keys,
    StdDuration::from_secs(5),
    5,
  );

  let mut input = String::new();
  loop {
    input.clear();
    io::stdin().read_line(&mut input).map_err(Error::StdIO)?;

    let command: Option<Command> = loop {
      if let Some((command, args)) = input.as_str().trim().split_once(' ') {
        let mut args_iter = args.trim_start().split_whitespace();
        match command {
          "REGISTER" => {
            if let (Some(name), Some(pass), None) =
              (args_iter.next(), args_iter.next(), args_iter.next())
            {
              let mut hasher = Sha256::new();
              hasher.update(pass.trim_start());
              let password = hasher.finalize().into();
              break Some(Command::Register {
                username: name.to_string(),
                password,
              });
            }
          }
          "LOGIN" => {
            if let (Some(name), Some(pass), None) =
              (args_iter.next(), args_iter.next(), args_iter.next())
            {
              let mut hasher = Sha256::new();
              hasher.update(pass.trim_start());
              let password = hasher.finalize().into();
              break Some(Command::Login {
                username: name.to_string(),
                password,
              });
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

              break Some(Command::ChangePassword { old, new });
            }
          }
          _ => {}
        }
      } else {
        // commands without argument
        match input.trim() {
          "GET_CHATROOM_STATUS" => {
            break Some(Command::GetChatroomStatus);
          }
          "LOGOUT" => {
            break Some(Command::Logout);
          }
          "HEARTBEAT" => {
            break Some(Command::Heartbeat);
          }
          _ => {}
        }
      }
      break None;
    };

    if let Some(command) = command {
      match command {
        command @ Command::Heartbeat => {
          connection
            .as_inner()
            .send_to_with_empty_meta(&command, server_addr)
            .await?;
          println!("[client] command sent");
        }
        command => {
          let respond = connection
            .request::<Command, Response>(&command, server_addr)
            .await?;
          println!("{:?}", respond);
        }
      }
    } else {
      eprintln!("[client] invalid command");
      continue;
    }
  }
}
