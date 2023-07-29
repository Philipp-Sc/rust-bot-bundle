pub mod chain_registry;
pub mod dummy;
pub mod fraud_detection;
pub mod governance;
pub mod staking;

use crate::plugin::store::fallback_entry_store::{FallbackEntryStore, RetrievalMethod};
use crate::PERSISTENT_SLED;
use cosmos_rust_package::api::custom::types::NextKeyType;
use std::sync::Arc;

static GLOBAL_PREFIX_TASK_STORE: &str = "task_store_";

lazy_static::lazy_static! {
    static ref AGENT_STORE: Arc<FallbackEntryStore> = Arc::new(FallbackEntryStore::new(&PERSISTENT_SLED.lock().unwrap().as_ref().unwrap(), GLOBAL_PREFIX_TASK_STORE));
}

type ContinueAtIndexType = Option<u64>;

pub fn get_next_key(continue_at_key: &str) -> Option<Vec<u8>> {
    let task_store = AGENT_STORE.clone();

    match task_store.get::<NextKeyType>(continue_at_key, &RetrievalMethod::Get) {
        Ok(item) => {
            if let Ok(key) = item.data {
                key
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

pub fn set_next_key(continue_at_key: &str, next_key: Option<Vec<u8>>) -> anyhow::Result<()> {
    let task_store = AGENT_STORE.clone();

    task_store.insert_if_not_exists::<NextKeyType>(&continue_at_key, Ok(next_key))?;
    Ok(())
}

pub fn get_next_index(continue_at_key: &str) -> Option<u64> {
    let task_store = AGENT_STORE.clone();

    match task_store.get::<ContinueAtIndexType>(continue_at_key, &RetrievalMethod::Get) {
        Ok(item) => {
            if let Ok(key) = item.data {
                key
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

pub fn set_next_index(continue_at_key: &str, next_index: Option<u64>) -> anyhow::Result<()> {
    let task_store = AGENT_STORE.clone();

    task_store.insert::<ContinueAtIndexType>(&continue_at_key, Ok(next_index))?;
    Ok(())
}
