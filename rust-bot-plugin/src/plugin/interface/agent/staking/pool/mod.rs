use crate::plugin::interface::agent::chain_registry::try_get_chain_registry;
use crate::plugin::interface::{Agent, TaskResult};
use crate::plugin::store::fallback_entry_store::{EntryError, RetrievalMethod};
use cosmos_rust_package::api::custom::types::PoolType;

use cosmos_rust_package::api::core::cosmos::channels::SupportedBlockchain;

use cosmos_rust_package::api::custom::query::staking::get_pool;

use log::info;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;

use crate::plugin::interface::agent::governance::GOVERNANCE_PREFIX;
use crate::plugin::interface::agent::AGENT_STORE;
use std::pin::Pin;

pub const POOL_PREFIX: &str = "pool_";

#[derive(Clone)]
pub struct PoolAgent {
    pub update_interval_in_secs: i64,
    pub retry_delay_in_secs: HashMap<PoolTasks,i64>,
    initial_retry_delay: i64,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct PoolTasks {
    blockchain_name: String,
}

impl Default for PoolAgent {
    fn default() -> Self {
        Self {
            update_interval_in_secs: 60 * 30,
            retry_delay_in_secs: HashMap::new(),
            initial_retry_delay: 60,
        }
    }
}

impl PoolAgent {
    async fn try_fetch_pool(
        _agent: PoolAgent,
        blockchain: SupportedBlockchain,
    ) -> anyhow::Result<()> {
        info!("now running: {:?}", (&blockchain.name));

        let task_store = AGENT_STORE.clone();

        let key = get_pool_entry_key(&blockchain);
        let result = get_pool(blockchain).await?;
        let data: Result<PoolType, EntryError> = Ok(result);

        task_store.insert_if_not_exists(&key, data)?;

        Ok(())
    }

    pub fn fetch_pool(
        &self,
        blockchain: SupportedBlockchain,
    ) -> Pin<Box<dyn Future<Output = TaskResult<PoolTasks>> + Send>> {
        let agent = self.clone();

        Box::pin(async move {
            TaskResult::new(
                PoolTasks {
                    blockchain_name: blockchain.name.to_owned(),
                },
                PoolAgent::try_fetch_pool(agent, blockchain).await,
            )
        })
    }
}

impl Agent for PoolAgent {
    type TaskType = PoolTasks;

    fn get_tasks(
        &self,
        tasks_pending: HashSet<Self::TaskType>,
    ) -> HashMap<
                Self::TaskType,
                Pin<Box<dyn Future<Output = TaskResult<Self::TaskType>> + Send>>,
            >
    {
        let self_clone = self.clone();

        let mut fns = HashMap::new();

        try_get_chain_registry().map(|x| {
            for (_, value) in x.into_iter() {
                let task_type = PoolTasks {
                    blockchain_name: value.name.to_owned(),
                };
                if !tasks_pending.iter().any(|x| x == &task_type) {
                    fns.insert(task_type, self_clone.fetch_pool(value.clone()));
                }
            }
        });
        fns
    }
    fn get_update_interval_in_secs(&self, _task_type: &Self::TaskType) -> i64 {
        self.update_interval_in_secs
    }
    fn get_retry_delay_in_secs(&self, task_type: &Self::TaskType) -> i64 {
        *self.retry_delay_in_secs.get(task_type).unwrap_or(&self.initial_retry_delay)
    }
    fn set_retry_delay_in_secs(&mut self, task_type: &Self::TaskType, retry_delay: i64) {
        self.retry_delay_in_secs.insert(task_type.clone(), retry_delay);
    }

    fn reset_retry_delay(&mut self, task_type: &Self::TaskType) {
        self.set_retry_delay_in_secs(task_type, self.initial_retry_delay);
    }
}

fn get_pool_entry_key(blockchain: &SupportedBlockchain) -> String {
    let key = format!("{}{}{}", GOVERNANCE_PREFIX, POOL_PREFIX, &blockchain.name);
    key
}

pub fn try_get_pool(blockchain: &SupportedBlockchain) -> Option<PoolType> {
    let task_store = AGENT_STORE.clone();

    let key = get_pool_entry_key(blockchain);

    match task_store.get::<PoolType>(&key, &RetrievalMethod::GetOk) {
        Ok(entry) => entry.data.ok(),
        Err(_) => None,
    }
}
