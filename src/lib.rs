// Copyright 2021 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

pub use error::Error;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::{collections::HashMap, path::PathBuf, sync::Mutex};
pub use store::{Store, StoreBuilder};
use tauri::{plugin::Plugin, AppHandle, Event, Invoke, Manager, Runtime, State, Window};

mod error;
mod store;

#[derive(Serialize, Clone)]
struct ChangePayload {
  path: PathBuf,
  key: String,
  value: JsonValue,
}

#[derive(Default)]
struct StoreCollection(Mutex<HashMap<PathBuf, Store>>);

fn with_store<R: Runtime, T, F: FnOnce(&mut Store) -> Result<T, Error>>(
  app: &AppHandle<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
  f: F,
) -> Result<T, Error> {
  let mut stores = stores.0.lock().expect("mutex poisoned");

  if !stores.contains_key(&path) {
    let mut store = StoreBuilder::new(path.clone()).build();
    // ignore loading errors, just use the default
    let _ = store.load(app);
    stores.insert(path.clone(), store);
  }

  f(stores
    .get_mut(&path)
    .expect("failed to retrieve store. This is a bug!"))
}

#[tauri::command]
async fn set<R: Runtime>(
  app: AppHandle<R>,
  window: Window<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
  key: String,
  value: JsonValue,
) -> Result<(), Error> {
  with_store(&app, stores, path.clone(), |store| {
    store.cache.insert(key.clone(), value.clone());
    let _ = window.emit("store://change", ChangePayload { path, key, value });
    Ok(())
  })
}

#[tauri::command]
async fn get<R: Runtime>(
  app: AppHandle<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
  key: String,
) -> Result<Option<JsonValue>, Error> {
  with_store(&app, stores, path, |store| {
    Ok(store.cache.get(&key).cloned())
  })
}

#[tauri::command]
async fn has<R: Runtime>(
  app: AppHandle<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
  key: String,
) -> Result<bool, Error> {
  with_store(&app, stores, path, |store| {
    Ok(store.cache.contains_key(&key))
  })
}

#[tauri::command]
async fn delete<R: Runtime>(
  app: AppHandle<R>,
  window: Window<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
  key: String,
) -> Result<bool, Error> {
  with_store(&app, stores, path.clone(), |store| {
    let flag = store.cache.remove(&key).is_some();
    if flag {
      let _ = window.emit(
        "store://change",
        ChangePayload {
          path,
          key,
          value: JsonValue::Null,
        },
      );
    }
    Ok(flag)
  })
}

#[tauri::command]
async fn clear<R: Runtime>(
  app: AppHandle<R>,
  window: Window<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
) -> Result<(), Error> {
  with_store(&app, stores, path.clone(), |store| {
    let keys = store.cache.keys().cloned().collect::<Vec<String>>();
    store.cache.clear();
    for key in keys {
      let _ = window.emit(
        "store://change",
        ChangePayload {
          path: path.clone(),
          key,
          value: JsonValue::Null,
        },
      );
    }
    Ok(())
  })
}

#[tauri::command]
async fn reset<R: Runtime>(
  app: AppHandle<R>,
  window: Window<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
) -> Result<(), Error> {
  let has_defaults = stores
    .0
    .lock()
    .expect("mutex poisoned")
    .get(&path)
    .map(|store| store.defaults.is_some());

  if Some(true) == has_defaults {
    with_store(&app, stores, path.clone(), |store| {
      if let Some(defaults) = &store.defaults {
        for (key, value) in &store.cache {
          if defaults.get(key) != Some(value) {
            let _ = window.emit(
              "store://change",
              ChangePayload {
                path: path.clone(),
                key: key.clone(),
                value: defaults.get(key).cloned().unwrap_or(JsonValue::Null),
              },
            );
          }
        }
        store.cache = defaults.clone();
      }
      Ok(())
    })
  } else {
    clear(app, window, stores, path).await
  }
}

#[tauri::command]
async fn keys<R: Runtime>(
  app: AppHandle<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
) -> Result<Vec<String>, Error> {
  with_store(&app, stores, path, |store| {
    Ok(store.cache.keys().cloned().collect())
  })
}

#[tauri::command]
async fn values<R: Runtime>(
  app: AppHandle<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
) -> Result<Vec<JsonValue>, Error> {
  with_store(&app, stores, path, |store| {
    Ok(store.cache.values().cloned().collect())
  })
}

#[tauri::command]
async fn entries<R: Runtime>(
  app: AppHandle<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
) -> Result<Vec<(String, JsonValue)>, Error> {
  with_store(&app, stores, path, |store| {
    Ok(store.cache.clone().into_iter().collect())
  })
}

#[tauri::command]
async fn length<R: Runtime>(
  app: AppHandle<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
) -> Result<usize, Error> {
  with_store(&app, stores, path, |store| Ok(store.cache.len()))
}

#[tauri::command]
async fn load<R: Runtime>(
  app: AppHandle<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
) -> Result<(), Error> {
  with_store(&app, stores, path, |store| store.load(&app))
}

#[tauri::command]
async fn save<R: Runtime>(
  app: AppHandle<R>,
  stores: State<'_, StoreCollection>,
  path: PathBuf,
) -> Result<(), Error> {
  with_store(&app, stores, path, |store| store.save(&app))
}

pub struct StorePlugin<R: Runtime> {
  invoke_handler: Box<dyn Fn(Invoke<R>) + Send + Sync>,
  stores: Option<HashMap<PathBuf, Store>>,
}

impl<R: Runtime> Default for StorePlugin<R> {
  fn default() -> Self {
    Self {
      invoke_handler: Box::new(tauri::generate_handler![
        set, get, has, delete, clear, reset, keys, values, length, entries, load, save
      ]),
      stores: None,
    }
  }
}

impl<R: Runtime> StorePlugin<R> {
  pub fn with_stores<T: IntoIterator<Item = Store>>(stores: T) -> Self {
    Self {
      invoke_handler: Box::new(tauri::generate_handler![
        set, get, has, delete, clear, reset, keys, values, length, entries, load, save
      ]),
      stores: Some(
        stores
          .into_iter()
          .map(|store| (store.path.clone(), store))
          .collect(),
      ),
    }
  }
}

impl<R: Runtime> Plugin<R> for StorePlugin<R> {
  fn name(&self) -> &'static str {
    "store"
  }

  fn extend_api(&mut self, message: Invoke<R>) {
    (self.invoke_handler)(message)
  }

  fn initialize(&mut self, app: &AppHandle<R>, _: JsonValue) -> tauri::plugin::Result<()> {
    app.manage(StoreCollection(Mutex::new(
      self.stores.clone().unwrap_or_default(),
    )));
    Ok(())
  }

  fn on_event(&mut self, app: &AppHandle<R>, event: &tauri::Event) {
    if let Event::Exit = event {
      let stores = app.state::<StoreCollection>();

      for store in stores.0.lock().expect("mutex poisoned").values() {
        if let Err(err) = store.save(app) {
          eprintln!("failed to save store {:?} with error {:?}", store.path, err);
        }
      }
    }
  }
}
