use crate::plugin::store::fallback_entry_store::EntryError;

use log::info;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use crate::plugin::interface::{Agent, TaskResult};

use crate::plugin::interface::agent::AGENT_STORE;
use cosmos_rust_package::api::core::cosmos::channels;
use cosmos_rust_package::api::core::cosmos::channels::SupportedBlockchainType;

use crate::plugin::store::fallback_entry_store::RetrievalMethod;

const STORE_KEY: &str = "chain_registry";

#[derive(Clone)]
pub struct ChainRegistryAgent {
    pub git_path: String,
    pub json_path: String,
    pub git_pull: bool,
    pub sync_interval_in_secs: Option<u64>,
    pub update_interval_in_secs: i64,
    pub retry_delay_in_secs: HashMap<ChainRegistryTasks,i64>,
    initial_retry_delay: i64,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub enum ChainRegistryTasks {
    FetchChainRegistry,
}

impl Default for ChainRegistryAgent {
    fn default() -> Self {
        Self {
            git_path: "./bin/assets/chain-registry".to_string(),
            json_path: "./bin/assets/supported_blockchains.json".to_string(),
            git_pull: false,             // TODO: PROD: true
            sync_interval_in_secs: None, // TODO: PROD: Some(60*60*1)
            update_interval_in_secs: 60 * 30,
            retry_delay_in_secs: HashMap::new(),
            initial_retry_delay: 60,
        }
    }
}

impl ChainRegistryAgent {
    async fn try_fetch_chain_registry(agent: ChainRegistryAgent) -> anyhow::Result<()> {
        info!("now running");
        // todo: this should return an iterator: Vec<HashMap<String,SupportedBlockchain>>
        // todo: this way as soon as one SupportedBlockchain is complete it can be saved (appended/updated) to the store.
        let result = channels::get_supported_blockchains_from_chain_registry(
            &agent.git_path,
            agent.git_pull,
            &agent.json_path,
            agent.sync_interval_in_secs,
        )
        .await?;
        info!("{:#?}", result.iter());

        let data: Result<SupportedBlockchainType, EntryError> = Ok(result);

        let task_store = AGENT_STORE.clone();
        task_store.insert(STORE_KEY, data)?;
        info!("done");
        Ok(())
    }

    fn fetch_chain_registry(
        &self,
    ) -> Pin<Box<dyn Future<Output = TaskResult<ChainRegistryTasks>> + Send>> {
        let agent = self.clone();

        Box::pin(async move {
            TaskResult::new(
                ChainRegistryTasks::FetchChainRegistry,
                ChainRegistryAgent::try_fetch_chain_registry(agent).await,
            )
        })
    }
}

impl Agent for ChainRegistryAgent {
    type TaskType = ChainRegistryTasks;

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
        if !tasks_pending
            .iter()
            .any(|x| x == &ChainRegistryTasks::FetchChainRegistry)
        {
            fns.insert(
                ChainRegistryTasks::FetchChainRegistry,
                self_clone.fetch_chain_registry(),
            );
        }
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

pub fn try_get_chain_registry() -> Option<SupportedBlockchainType> {
    let task_store = AGENT_STORE.clone();

    match task_store.get::<SupportedBlockchainType>(STORE_KEY, &RetrievalMethod::GetOk) {
        Ok(item) => {
            if let Ok(data) = item.data {
                Some(data)
            } else {
                None
            }
        }
        Err(_) => None,
    }
}
