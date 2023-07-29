use crate::plugin::interface::agent::chain_registry::try_get_chain_registry;
use crate::plugin::interface::{Agent, TaskResult};
use crate::plugin::store::fallback_entry_store::{EntryError, RetrievalMethod};

use cosmos_rust_package::api::core::cosmos::channels::SupportedBlockchain;
use cosmos_rust_package::api::custom::query::gov::get_params_v1beta1;

use log::info;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;

use crate::plugin::interface::agent::governance::GOVERNANCE_PREFIX;
use crate::plugin::interface::agent::AGENT_STORE;
use cosmos_rust_package::api::custom::types::ParamsType;
use std::pin::Pin;

pub const PARAMS_PREFIX: &str = "params_";

#[derive(Clone)]
pub struct ParamsAgent {
    pub params_types: Vec<String>,
    pub update_interval_in_secs: i64,
    pub retry_delay_in_secs: HashMap<ParamsTasks,i64>,
    initial_retry_delay: i64,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct ParamsTasks {
    blockchain_name: String,
    params_type: String,
}

impl Default for ParamsAgent {
    fn default() -> Self {
        Self {
            params_types: vec![
                "voting".to_string(),
                "tallying".to_string(),
                "deposit".to_string(),
            ],
            update_interval_in_secs: 60 * 60, // 1h
            retry_delay_in_secs: HashMap::new(),
            initial_retry_delay: 60,
        }
    }
}

impl ParamsAgent {
    async fn try_fetch_params(
        _agent: ParamsAgent,
        blockchain: SupportedBlockchain,
        params_type: String,
    ) -> anyhow::Result<()> {
        info!("now running: {:?}", (&blockchain.name, &params_type));

        let task_store = AGENT_STORE.clone();

        let result = get_params_v1beta1(blockchain.clone(), params_type.clone()).await;

        let mut output: Result<(), anyhow::Error> = Ok(());
        let data: Result<ParamsType, EntryError> = match result {
            Ok(params) => Ok(params),
            Err(err) => {
                output = Err(err);
                Err(EntryError::Error(
                    output.as_ref().err().unwrap().to_string(),
                ))
            }
        };

        let key = get_params_entry_key(&blockchain, &params_type);
        task_store.insert_if_not_exists(&key, data)?;

        output
    }

    pub fn fetch_params(
        &self,
        blockchain: SupportedBlockchain,
        params_type: String,
    ) -> Pin<Box<dyn Future<Output = TaskResult<ParamsTasks>> + Send>> {
        let agent = self.clone();

        Box::pin(async move {
            TaskResult::new(
                ParamsTasks {
                    blockchain_name: blockchain.name.to_owned(),
                    params_type: params_type.clone(),
                },
                ParamsAgent::try_fetch_params(agent, blockchain, params_type).await,
            )
        })
    }
}

impl Agent for ParamsAgent {
    type TaskType = ParamsTasks;

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
                for params_type in &self_clone.params_types {
                    let task_type = ParamsTasks {
                        blockchain_name: value.name.to_owned(),
                        params_type: params_type.clone(),
                    };
                    if !tasks_pending.iter().any(|x| x == &task_type) {
                        fns.insert(
                            task_type,
                            self_clone.fetch_params(value.clone(), params_type.to_string()),
                        );
                    }
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

fn get_params_entry_key(blockchain: &SupportedBlockchain, params_type: &str) -> String {
    let key = format!(
        "{}{}{}_{}",
        GOVERNANCE_PREFIX, PARAMS_PREFIX, &blockchain.name, params_type
    );
    key
}

pub fn try_get_params(blockchain: &SupportedBlockchain, params_type: &str) -> Option<ParamsType> {
    let task_store = AGENT_STORE.clone();

    let key = get_params_entry_key(blockchain, params_type);

    match task_store.get::<ParamsType>(&key, &RetrievalMethod::GetOk) {
        Ok(entry) => entry.data.ok(),
        Err(_) => None,
    }
}

/*if let Some(tonic_status) = err.downcast_ref::<tonic::Status>() {
    if tonic_status.code() == tonic::Code::Unavailable {
        for each in tonic_status.metadata().get_all("ratelimit-policy"){

            // Split the string into two parts based on the semicolon
            let parts: Vec<&str> = each.to_str().unwrap_or("").split(';').collect();

            if let Some(rate_limit) = parts.get(0) {
                let rate_limit_value: u64 = rate_limit.trim().parse().unwrap_or(0);
                error!("Rate Limit: {}", rate_limit_value);
            }

            if let Some(window) = parts.get(1) {
                // Split the second part based on the equal sign to get the window value
                let window_value: u64 = window.split('=').nth(1).unwrap_or("0").trim().parse().unwrap_or(0);
                error!("Window: {}", window_value);
            }
        }
    }
}else */
