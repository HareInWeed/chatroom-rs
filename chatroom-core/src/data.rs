use time::OffsetDateTime;

use std::net::SocketAddr;

use thiserror::Error;

use serde::{Deserialize, Serialize};

#[allow(deprecated)]
use bincode::{config, DefaultOptions, Options};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct User {
  pub name: String,
  pub password_hash: String,
  pub ip_address: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UserInfo {
  pub name: String,
  pub ip_address: SocketAddr,
  pub is_online: bool,
}

impl UserInfo {
  pub fn new(user: &User, is_online: bool) -> Self {
    let User {
      name, ip_address, ..
    } = user.clone();
    Self {
      name,
      ip_address,
      is_online,
    }
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
  Offline(String),
  Online(UserInfo),
}

#[derive(Error, Clone, Debug, PartialEq, Deserialize, Serialize)]
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

pub fn default_coder() -> config::WithOtherEndian<
  config::WithOtherTrailing<
    config::WithOtherIntEncoding<config::DefaultOptions, config::FixintEncoding>,
    config::AllowTrailing,
  >,
  config::BigEndian,
> {
  DefaultOptions::new()
    .with_fixint_encoding()
    .allow_trailing_bytes()
    .with_big_endian()
}
