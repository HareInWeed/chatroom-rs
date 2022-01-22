#![cfg_attr(
  all(not(debug_assertions), target_os = "windows"),
  windows_subsystem = "windows"
)]

use tauri::AppHandle;
use tracing_subscriber::{fmt, prelude::*};

use tracing::{info, instrument};

mod server;
mod utils;

use std::{iter, sync::Arc};

use server::Server;

use chatroom_core::{
  data::{default_coder, DefaultCoder, User},
  utils::{Error, ErrorMsg},
};

use parking_lot::RwLock;

use std::time::Duration as StdDuration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Settings {
  heartbeat_interval: StdDuration,
  server_addr: String,
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      heartbeat_interval: StdDuration::from_secs(60),
      server_addr: "0.0.0.0:0".into(),
    }
  }
}

#[derive(Default)]
struct State {
  settings: RwLock<Settings>,
  server: RwLock<Option<Server<DefaultCoder>>>,
}

type MyState = Arc<State>;

#[tauri::command]
#[instrument(skip(app, state))]
async fn start_server(app: AppHandle, state: tauri::State<'_, MyState>) -> Result<(), ErrorMsg> {
  let Settings {
    heartbeat_interval,
    server_addr,
  } = state.settings.read().clone();
  stop_server(state.clone()).await?;
  let server = Server::new(
    default_coder(),
    iter::empty(),
    app.clone(),
    heartbeat_interval,
    &server_addr,
  )
  .await;
  match server {
    Ok(server) => {
      *state.server.write() = Some(server);
      Ok(())
    }
    Err(ref err @ Error::Network(ref inner)) => {
      if matches!(inner.raw_os_error(), Some(10048)) {
        info!(
          source = "server",
          "address \"{}\" is already bound.", server_addr
        );
      }
      Err(err.into())
    }
    Err(err) => Err(err.into()),
  }
}

#[tauri::command]
#[instrument(skip(state))]
async fn stop_server(state: tauri::State<'_, MyState>) -> Result<(), ErrorMsg> {
  let _ = state.server.write().take();
  Ok(())
}

#[tauri::command]
#[instrument(skip(state))]
async fn get_users(state: tauri::State<'_, MyState>) -> Result<Vec<User>, ErrorMsg> {
  if let Some(server) = state.server.read().as_ref() {
    Ok(
      server
        .get_state()
        .users
        .read()
        .values()
        .map(|u| u.clone())
        .collect(),
    )
  } else {
    Ok(vec![])
  }
}

#[tauri::command]
#[instrument(skip(state))]
async fn is_server_on(state: tauri::State<'_, MyState>) -> Result<bool, ErrorMsg> {
  Ok(state.server.read().is_some())
}

#[tauri::command]
#[instrument(skip(state))]
async fn get_settings(state: tauri::State<'_, MyState>) -> Result<Settings, ErrorMsg> {
  Ok(state.settings.read().clone())
}

#[tauri::command]
#[instrument(skip(state))]
async fn set_settings(
  state: tauri::State<'_, MyState>,
  heartbeat_interval: Option<u64>,
  server_addr: Option<String>,
) -> Result<(), ErrorMsg> {
  let mut settings = state.settings.write();
  if let Some(heartbeat_interval) = heartbeat_interval {
    settings.heartbeat_interval = StdDuration::from_millis(heartbeat_interval);
  };
  if let Some(server_addr) = server_addr {
    settings.server_addr = server_addr;
  };
  Ok(())
}

fn main() {
  tauri::Builder::default()
    .manage(MyState::default())
    .invoke_handler(tauri::generate_handler![
      start_server,
      stop_server,
      get_users,
      get_settings,
      set_settings,
      is_server_on
    ])
    .setup(|app| {
      let subscriber = fmt()
        .with_writer(utils::LogWriterMaker::new(app.handle()))
        .with_ansi(false)
        .with_target(false)
        .with_timer(fmt::time::LocalTime::rfc_3339())
        .finish();
      tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
