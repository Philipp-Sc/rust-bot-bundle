use crate::plugin::interface::agent::chain_registry::try_get_chain_registry;
use crate::plugin::interface::agent::{get_next_key, set_next_key, AGENT_STORE};
use crate::plugin::interface::{Agent, TaskResult};

use cosmos_rust_package::api::core::cosmos::channels::SupportedBlockchain;
use cosmos_rust_package::api::custom::query::gov::get_validators_v1beta1;

use log::info;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;

use cosmos_rust_package::api::custom::types::ValidatorsType;
use std::pin::Pin;

use crate::plugin::interface::agent::governance::GOVERNANCE_PREFIX;

pub const VALIDATOR_PREFIX: &str = "validator_";

#[derive(Clone)]
pub struct ValidatorsAgent {
    pub continue_at_key_prefix: String,
    pub update_interval_in_secs: i64,
    pub retry_delay_in_secs: HashMap<ValidatorsTasks,i64>,
    initial_retry_delay: i64,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct ValidatorsTasks {
    blockchain_name: String,
}

impl Default for ValidatorsAgent {
    fn default() -> Self {
        Self {
            continue_at_key_prefix: "fetch_validators_for".to_string(),
            update_interval_in_secs: 60 * 60, // 1h
            retry_delay_in_secs: HashMap::new(),
            initial_retry_delay: 60,
        }
    }
}

impl ValidatorsAgent {
    async fn try_fetch_validators(
        agent: ValidatorsAgent,
        blockchain: SupportedBlockchain,
    ) -> anyhow::Result<()> {
        info!("now running: {:?}", (&blockchain.name));

        let task_store = AGENT_STORE.clone();

        let continue_at_key = format!("{}_{}", agent.continue_at_key_prefix, blockchain.name);

        let mut next_key = get_next_key(&continue_at_key);

        let result = get_validators_v1beta1(blockchain.clone(), next_key.clone()).await;

        if let Ok(validators) = result {
            for validator in validators.1 {
                let validator_key = format!(
                    "{}{}{}_{}",
                    GOVERNANCE_PREFIX,
                    VALIDATOR_PREFIX,
                    blockchain.name,
                    validator.object_to_hash()
                );
                task_store.insert_if_not_exists::<ValidatorsType>(&validator_key, Ok(validator))?;
            }

            next_key = validators.0.clone();

            if let Some(ref new_next_key) = next_key {
                if new_next_key.is_empty() {
                    // vec![]
                    // removing continue key
                    task_store.remove_all(&continue_at_key)?;
                    return Ok(());
                }
                // continue with valid pagination response for next key.
                // save next_key (checkpoint)
                set_next_key(&continue_at_key, next_key.clone())?;
            } else {
                // no pagination response | no next key
                // removing continue key
                task_store.remove_all(&continue_at_key)?;
                return Ok(());
            }
        } else {
            // might return unavailable due to rate-limiting policy
            // which makes starting at the beginning over and over inefficient
            // therefore saving continue key
            set_next_key(&continue_at_key, next_key.clone())?;
            result?;
        }
        Ok(())
    }

    pub fn fetch_validators(
        &self,
        blockchain: SupportedBlockchain,
    ) -> Pin<Box<dyn Future<Output = TaskResult<ValidatorsTasks>> + Send>> {
        let agent = self.clone();

        Box::pin(async move {
            TaskResult::new(
                ValidatorsTasks {
                    blockchain_name: blockchain.name.to_owned(),
                },
                ValidatorsAgent::try_fetch_validators(agent, blockchain).await,
            )
        })
    }
}

impl Agent for ValidatorsAgent {
    type TaskType = ValidatorsTasks;

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
                let task_type = ValidatorsTasks {
                    blockchain_name: value.name.to_owned(),
                };
                if !tasks_pending.iter().any(|x| x == &task_type) {
                    fns.insert(task_type, self_clone.fetch_validators(value.clone()));
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
