mod plugin;

use std::sync::Once;
use std::sync::{Arc, Mutex, RwLock};

use env_logger::{Builder, Env};
use log::{error, info};

#[cfg(feature = "ChainRegistry")]
use crate::plugin::interface::agent::chain_registry::ChainRegistryAgent;
#[cfg(feature = "Dummy")]
use crate::plugin::interface::agent::dummy::DummyAgent;
#[cfg(feature = "FraudDetection")]
use crate::plugin::interface::agent::fraud_detection::FraudDetectionAgent;
#[cfg(feature = "Params")]
use crate::plugin::interface::agent::governance::params::ParamsAgent;
#[cfg(feature = "GovernanceProposalFetch")]
use crate::plugin::interface::agent::governance::proposals::fetch::GovernanceProposalFetchAgent;
#[cfg(feature = "GovernanceProposalView")]
use crate::plugin::interface::agent::governance::proposals::update::GovernanceProposalViewAgent;
#[cfg(feature = "TallyResults")]
use crate::plugin::interface::agent::governance::tally_results::TallyResultsAgent;
#[cfg(feature = "Validators")]
use crate::plugin::interface::agent::governance::validators::ValidatorsAgent;
#[cfg(feature = "Pool")]
use crate::plugin::interface::agent::staking::pool::PoolAgent;
use crate::plugin::interface::{Agent, TaskResult, AgentManager};
use chrono::Utc;
use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::Hash;
use tokio::runtime::Runtime;

use tokio::task::JoinSet;

lazy_static::lazy_static! {
    static ref PERSISTENT_SLED: Mutex<Option<sled::Db>> = Mutex::new(None);
    static ref TEMPORARY_SLED: Mutex<Option<sled::Db>> = Mutex::new(None);
    static ref RT: Arc<RwLock<Option<Runtime>>> = Arc::new(RwLock::new(Runtime::new().ok()));
    static ref CANCELLATION_FLAG: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
}

static INIT: Once = Once::new();

pub enum TaskState {
    Pending(tokio::task::Id),
    Cancelled(i64),
    Panicked(i64),
    Failed(i64),
    Resolved(i64),
}

#[no_mangle]
pub fn init(persistent_sled: &sled::Db, temporary_sled: &sled::Db) {
    #[cfg(feature = "EnvLogger")]
    {
        Builder::from_env(Env::default().default_filter_or("info"))
            .filter_module(
                "rust_bot_plugin::plugin::store::fallback_entry_store",
                log::LevelFilter::Off,
            )
            .filter_module(
                "cosmos_rust_package::api::core::cosmos::channels",
                log::LevelFilter::Off,
            )
            .filter_module("crate::plugin::stores", log::LevelFilter::Off)
            .format_timestamp(None)
            .format_module_path(false)
            .init();
    }
    info!("Init called");
    INIT.call_once(|| {
        let mut persistent = PERSISTENT_SLED.lock().unwrap();
        let mut temporary = TEMPORARY_SLED.lock().unwrap();

        if persistent.is_none() && temporary.is_none() {
            *persistent = Some(persistent_sled.clone());
            *temporary = Some(temporary_sled.clone());
        }
        info!("Init completed");
    });
}

#[no_mangle]
pub fn start() {
    if INIT.is_completed() {
        #[cfg(feature = "ChainRegistry")]
        {
            let agent: Box<dyn Agent<TaskType = _>> = Box::new(ChainRegistryAgent::default());
            let _join_handle = AgentManager::new(agent).run();
        }

        #[cfg(feature = "Params")]
        {
            let agent: Box<dyn Agent<TaskType = _>> = Box::new(ParamsAgent::default());
            let _join_handle = AgentManager::new(agent).run();
        }

        #[cfg(feature = "TallyResults")]
        {
            let agent: Box<dyn Agent<TaskType = _>> = Box::new(TallyResultsAgent::default());
            let _join_handle = AgentManager::new(agent).run();
        }

        #[cfg(feature = "Pool")]
        {
            let agent: Box<dyn Agent<TaskType = _>> = Box::new(PoolAgent::default());
            let _join_handle = AgentManager::new(agent).run();
        }

        #[cfg(feature = "FraudDetection")]
        {
            let agent: Box<dyn Agent<TaskType = _>> = Box::new(FraudDetectionAgent::default());
            let _join_handle = AgentManager::new(agent).run();
        }

        #[cfg(feature = "Validators")]
        {
            let agent: Box<dyn Agent<TaskType = _>> = Box::new(ValidatorsAgent::default());
            let _join_handle = AgentManager::new(agent).run();
        }

        #[cfg(feature = "GovernanceProposalFetch")]
        {
            let agent: Box<dyn Agent<TaskType = _>> =
                Box::new(GovernanceProposalFetchAgent::default());
            let _join_handle = AgentManager::new(agent).run();
        }

        #[cfg(feature = "GovernanceProposalView")]
        {
            let agent: Box<dyn Agent<TaskType = _>> =
                Box::new(GovernanceProposalViewAgent::default());
            let _join_handle = AgentManager::new(agent).run();
        }

        #[cfg(feature = "Dummy")]
        {
            let agent: Box<dyn Agent<TaskType = _>> = Box::new(DummyAgent::default());
            let _join_handle = AgentManager::new(agent).run();
        }
    } else {
        error!("Plugin not yet initialized");
    }
}


#[no_mangle]
pub fn stop() {
    info!("stop called");
    CANCELLATION_FLAG.store(true, Ordering::SeqCst);
    info!("goodbye!");
}

#[no_mangle]
pub fn shutdown() {
    info!("shutdown called");
    CANCELLATION_FLAG.store(true, Ordering::SeqCst);

    let rt_clone = RT.clone();
    // Obtain a mutable reference to the `Option<Runtime>` inside the `Mutex`
    let runtime: &mut Option<Runtime> = &mut rt_clone.write().unwrap();
    // Create a placeholder to replace the existing `Runtime`
    let replacement: Option<Runtime> = None;
    // Replace the existing `Runtime` with the placeholder and obtain the previous value
    if let Some(runtime) = std::mem::replace(runtime, replacement) {
        // Perform shutdown
        runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    }

    info!("goodbye for good!");
}
