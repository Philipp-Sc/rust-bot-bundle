


use libloading::{Library, Symbol};
use async_trait::async_trait;

use std::sync::{Arc, Mutex};
use notify::{RecommendedWatcher, RecursiveMode, Watcher,recommended_watcher, EventKind};

use std::collections::HashMap;
use std::path::Path;
use log::{info,error};



lazy_static::lazy_static! {
    static ref PERSISTENT_SLED: sled::Db = load_sled_db("./bin/tmp/persistent_sled");
    static ref TEMPORARY_SLED: sled::Db = load_sled_db("./bin/tmp/temporary_sled");
}

fn load_sled_db(path: &str) -> sled::Db {
    sled::Config::default()
        .path(path)
        .cache_capacity(1024 * 1024 * 1024)
        .flush_every_ms(Some(1000))
        .mode(sled::Mode::HighThroughput)
        .open()
        .unwrap()
}

type InitFn = fn(&sled::Db, &sled::Db);
type StartFn = fn();
type StopFn = fn();
type ShutdownFn = fn();

#[async_trait]
trait Plugin {
    fn init(&self, persistent_sled: &sled::Db, temporary_sled: &sled::Db);
    fn start(&self);
    fn stop(&self);
    fn shutdown(&self);
}

struct DefaultPlugin {
    library: Library,
}

impl DefaultPlugin {
    fn new(library_path: &str) -> anyhow::Result<Self> {
        unsafe {
            let library = Library::new(library_path)?;
            Ok(DefaultPlugin { library })
        }
    }
}
#[async_trait]
impl Plugin for DefaultPlugin {
    fn init(&self, persistent_sled: &sled::Db, temporary_sled: &sled::Db) {
        unsafe {
            let init_fn: Symbol<InitFn> = self.library.get(b"init\0").unwrap();
            init_fn(persistent_sled, temporary_sled);
        }
    }

    fn start(&self) {
        unsafe {
            let start_fn: Symbol<StartFn> = self.library.get(b"start\0").unwrap();
            start_fn()
        }

    }

    fn stop(&self) {
        unsafe {
            let stop_fn: Symbol<StopFn> = self.library.get(b"stop\0").unwrap();
            stop_fn()
        }
    }
    fn shutdown(&self) {
        unsafe {
            let shutdown_fn: Symbol<ShutdownFn> = self.library.get(b"shutdown\0").unwrap();
            shutdown_fn()
        }
    }
}


#[tokio::main]
async fn main() {

    // Initialize the logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let library_path = "./bin/lib/";
    let active_plugins: Arc<Mutex<HashMap<String, DefaultPlugin>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Load and start all libraries initially
    let library_files = std::fs::read_dir(&library_path)
        .expect("Failed to read library directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.path().extension().map(|ext| ext == "so").unwrap_or(false)
        });

    for entry in library_files {
        let path = entry.path();
        let path_str = path.to_str().unwrap();
        info!("Starting library: {}", path_str);

        let mut plugins = active_plugins.lock().unwrap();

        if let Some(_plugin) = plugins.get_mut(path_str) {
            // If the plugin is already running, skip starting it again
            continue;
        }

        let new_plugin = DefaultPlugin::new(path_str).expect("Failed to load library");
        new_plugin.init(&PERSISTENT_SLED, &TEMPORARY_SLED);
        new_plugin.start();

        plugins.insert(path_str.to_string(), new_plugin);
    }

    // Start the file watcher after initializing libraries
    let mut watcher: RecommendedWatcher = recommended_watcher(move |res: notify::Result<notify::Event>| {
        match res {
            Ok(event) => {
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) => {
                        let path = &event.paths[0];
                        if let Some(extension) = path.extension() {
                            if extension == "so" {
                                let path_str = path.to_str().unwrap();
                                info!("Detected change in: {}", path_str);

                                let mut plugins = active_plugins.lock().unwrap();

                                if let Some(plugin) = plugins.get_mut(path_str) {
                                    // if the plugin is already running, shut it down
                                    plugin.shutdown();
                                    info!("Re-starting library: {}", path_str);
                                }else{
                                    info!("Starting library: {}", path_str);
                                }

                                let new_plugin = DefaultPlugin::new(path_str).expect("Failed to load library");


                                new_plugin.init(&PERSISTENT_SLED, &TEMPORARY_SLED);
                                new_plugin.start();

                                plugins.insert(path_str.to_string(), new_plugin);
                            }
                        }
                    },
                    EventKind::Remove(_) => {
                        let path_str = event.paths[0].to_str().unwrap();
                        info!("Detected change in: {}", path_str);

                        let mut plugins = active_plugins.lock().unwrap();
                        if let Some(plugin) = plugins.remove(path_str) {
                            // if the plugin is being removed, shut it down
                            info!("Shutting down library: {}", path_str);
                            plugin.shutdown();

                        }
                    },
                    _ => {}
                }
            }
            Err(err) => {
                error!("Error while watching for file changes: {:?}", err);
            }
        }
    }).unwrap();

    watcher.watch(Path::new(&library_path), RecursiveMode::NonRecursive).unwrap();

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
