use time::OffsetDateTime;

use std::net::SocketAddr;

use thiserror::Error as ThisError;

use serde::{Deserialize, Serialize};

#[allow(deprecated)]
use bincode::{config, DefaultOptions, Error as BinCodeError, Options};

use byteorder::{ByteOrder, NetworkEndian};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum SecureMsg {
  MyKey([u8; 32]),
  PeerKey([u8; 32]),
  Msg(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct User {
  pub name: String,
  pub password_hash: String,
  pub online_info: Option<UserOnlineInfo>,
}

impl From<(String, UserEssential)> for User {
  fn from(data: (String, UserEssential)) -> Self {
    let (name, UserEssential { password_hash }) = data;
    Self {
      name,
      password_hash,
      online_info: None,
    }
  }
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UserEssential {
  pub password_hash: String,
}

impl From<&User> for UserEssential {
  fn from(data: &User) -> Self {
    let User { password_hash, .. } = data;
    Self {
      password_hash: password_hash.clone(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UserOnlineInfo {
  pub ip_address: SocketAddr,
  pub pub_key: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UserInfo {
  pub name: String,
  pub online_info: Option<UserOnlineInfo>,
}

impl UserInfo {
  pub fn new(user: &User) -> Self {
    let User {
      name, online_info, ..
    } = user.clone();
    Self { name, online_info }
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Command {
  Register {
    username: String,
    password: [u8; 32],
  },
  Login {
    username: String,
    password: [u8; 32],
  },
  ChangePassword {
    old: [u8; 32],
    new: [u8; 32],
  },
  GetChatroomStatus,
  Heartbeat,
  Logout,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub enum ResponseData {
  Success,
  ChatroomStatus { users: Vec<UserInfo> },
}

pub type Response = Result<ResponseData, ErrorCode>;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Notification {
  Online {
    timestamp: OffsetDateTime,
    name: String,
    info: UserOnlineInfo,
  },
  Offline {
    timestamp: OffsetDateTime,
    name: String,
  },
}

#[derive(ThisError, Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub enum ErrorCode {
  // register
  #[error("user is already existed")]
  UserExisted,
  // login
  #[error("username or password are invalid")]
  InvalidUserOrPass,
  // login
  #[error("login is required for the operation")]
  LoginRequired,
  // secure
  #[error("failed to establish a secure connection")]
  ConnectionNotSecure,
  // general
  #[error("operation is not supported")]
  Unsupported,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Message {
  pub to_all: bool,
  pub timestamp: OffsetDateTime,
  pub msg: String,
}

pub type DefaultCoder = config::WithOtherEndian<
  config::WithOtherTrailing<
    config::WithOtherIntEncoding<config::DefaultOptions, config::FixintEncoding>,
    config::AllowTrailing,
  >,
  config::BigEndian,
>;

pub fn default_coder() -> DefaultCoder {
  DefaultOptions::new()
    .with_fixint_encoding()
    .allow_trailing_bytes()
    .with_big_endian()
}

pub fn serialize_with_meta<C, T>(coder: C, data: &T, id: u16) -> Result<Vec<u8>, BinCodeError>
where
  C: Options,
  T: Serialize,
{
  let mut buf = vec![0u8; 2];
  NetworkEndian::write_u16(&mut buf[..], id);
  coder.serialize_into(&mut buf, data)?;
  Ok(buf)
}
