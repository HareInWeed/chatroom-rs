#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

mod client;

use std::{net::SocketAddr, sync::Arc};

use client::{ChatEntry, Client, OwnedChatEntry, PersonalInfo};

use chatroom_core::{
  data::{default_coder, DefaultCoder, ErrorCode, UserInfo},
  utils::ErrorMsg,
};

use parking_lot::RwLock;

use time::{OffsetDateTime, UtcOffset};
use tokio::sync::RwLock as ArwLock;

use std::time::Duration as StdDuration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Settings {
  heartbeat_interval: StdDuration,
  server_addr: String,
  client_addr: String,
  request_timeout: StdDuration,
  retry_limits: u32,
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      heartbeat_interval: StdDuration::from_secs(30),
      server_addr: "0.0.0.0:0".into(),
      client_addr: "0.0.0.0:0".into(),
      request_timeout: StdDuration::from_secs(5),
      retry_limits: 5,
    }
  }
}

#[derive(Default)]
struct State {
  settings: RwLock<Settings>,
  client: ArwLock<Option<Client<DefaultCoder>>>,
}

type MyState = Arc<State>;

#[tauri::command]
async fn connect_server(
  app: tauri::AppHandle,
  state: tauri::State<'_, MyState>,
  server_addr: String,
) -> Result<(), ErrorMsg> {
  let server_addr_str = server_addr;
  let server_addr = server_addr_str.parse::<SocketAddr>()?;

  disconnect_server(state.clone()).await?;
  let Settings {
    heartbeat_interval,
    client_addr,
    request_timeout,
    retry_limits,
    ..
  } = {
    let mut settings = state.settings.write();
    settings.server_addr = server_addr_str;
    settings.clone()
  };

  let client_addr = client_addr.parse::<SocketAddr>()?;
  let client = Client::new(
    client_addr,
    server_addr,
    app,
    default_coder(),
    heartbeat_interval,
    request_timeout,
    retry_limits,
  )
  .await?;
  *state.client.write().await = Some(client);
  Ok(())
}

#[tauri::command]
async fn disconnect_server(state: tauri::State<'_, MyState>) -> Result<(), ErrorMsg> {
  let mut client = state.client.write().await;
  if let Some(c) = client.take() {
    if let Some(old) = c.logout().await? {
      *client = Some(old);
      return Err("failed to logout".into());
    }
  }
  Ok(())
}

#[tauri::command]
async fn register(
  state: tauri::State<'_, MyState>,
  username: String,
  password: String,
) -> Result<(), ErrorMsg> {
  let client = state.client.read().await;
  if let Some(client) = client.as_ref() {
    Ok(client.register(username, password.as_str()).await?)
  } else {
    Err("server not connected".into())
  }
}

#[tauri::command]
async fn get_server_info(state: tauri::State<'_, MyState>) -> Result<Option<SocketAddr>, ErrorMsg> {
  let client = state.client.read().await;
  if let Some(client) = client.as_ref() {
    Ok(Some(client.server_addr))
  } else {
    Ok(None)
  }
}

#[tauri::command]
async fn login(
  state: tauri::State<'_, MyState>,
  username: String,
  password: String,
) -> Result<(), ErrorMsg> {
  let client = state.client.read().await;
  if let Some(client) = client.as_ref() {
    Ok(client.login(username, password.as_str()).await?)
  } else {
    Err("server not connected".into())
  }
}

#[tauri::command]
async fn change_password(
  state: tauri::State<'_, MyState>,
  old: String,
  new: String,
) -> Result<(), ErrorMsg> {
  let client = state.client.read().await;
  if let Some(client) = client.as_ref() {
    Ok(client.change_password(old.as_str(), new.as_str()).await?)
  } else {
    Err("server not connected".into())
  }
}

#[tauri::command]
async fn say(
  state: tauri::State<'_, MyState>,
  username: Option<String>,
  msg: String,
) -> Result<(), ErrorMsg> {
  let client = state.client.read().await;
  if let Some(client) = client.as_ref() {
    Ok(client.say(msg, username).await?)
  } else {
    Err("server not connected".into())
  }
}

#[tauri::command]
async fn fetch_chatroom_status(state: tauri::State<'_, MyState>) -> Result<(), ErrorMsg> {
  let client = state.client.read().await;
  if let Some(client) = client.as_ref() {
    Ok(client.fetch_chatroom_status().await?)
  } else {
    Err("server not connected".into())
  }
}

#[tauri::command]
async fn logout(state: tauri::State<'_, MyState>) -> Result<(), ErrorMsg> {
  disconnect_server(state).await
}

#[tauri::command]
async fn get_personal_info(state: tauri::State<'_, MyState>) -> Result<PersonalInfo, ErrorMsg> {
  let client = state.client.read().await;
  if let Some(client) = client.as_ref() {
    let info = client.get_state().personal_info.lock().clone();
    if let Some(info) = info {
      Ok(info)
    } else {
      Err(ErrorCode::LoginRequired.into())
    }
  } else {
    Err("server not connected".into())
  }
}

#[tauri::command]
async fn get_user_info(state: tauri::State<'_, MyState>) -> Result<Vec<UserInfo>, ErrorMsg> {
  let client = state.client.read().await;
  if let Some(client) = client.as_ref() {
    Ok(
      client
        .get_state()
        .users
        .read()
        .values()
        .map(|i| i.clone())
        .collect(),
    )
  } else {
    Err("server not connected".into())
  }
}

#[tauri::command]
async fn get_chats(
  state: tauri::State<'_, MyState>,
  name: Option<String>,
) -> Result<Vec<(OffsetDateTime, OwnedChatEntry)>, ErrorMsg> {
  let client = state.client.read().await;
  if let Some(client) = client.as_ref() {
    if let Some(name) = name {
      if let Some(history) = client.get_state().ono2one_history.read().get(&name) {
        let offset = match UtcOffset::current_local_offset() {
          Ok(offset) => offset,
          Err(_) => UtcOffset::UTC,
        };
        Ok(
          history
            .iter()
            .map(|(t, c)| (t.clone().to_offset(offset.clone()), c.clone()))
            .collect(),
        )
      } else {
        Err(ErrorCode::UserNotExisted.into())
      }
    } else {
      let offset = match UtcOffset::current_local_offset() {
        Ok(offset) => offset,
        Err(_) => UtcOffset::UTC,
      };
      Ok(
        client
          .get_state()
          .group_history
          .read()
          .iter()
          .map(|(t, c)| (t.clone().to_offset(offset.clone()), c.clone()))
          .collect(),
      )
    }
  } else {
    Err("server not connected".into())
  }
}

fn main() {
  tauri::Builder::default()
    .manage(MyState::default())
    .invoke_handler(tauri::generate_handler![
      get_server_info,
      connect_server,
      disconnect_server,
      register,
      login,
      change_password,
      say,
      fetch_chatroom_status,
      logout,
      get_personal_info,
      get_user_info,
      get_chats,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
