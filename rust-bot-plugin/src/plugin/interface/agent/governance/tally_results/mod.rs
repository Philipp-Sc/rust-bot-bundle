use crate::plugin::interface::agent::chain_registry::try_get_chain_registry;
use crate::plugin::interface::agent::{get_next_index, set_next_index, AGENT_STORE};
use crate::plugin::interface::{Agent, TaskResult};
use crate::plugin::store::fallback_entry_store::{EntryError, RetrievalMethod};

use cosmos_rust_package::api::core::cosmos::channels::SupportedBlockchain;
use cosmos_rust_package::api::custom::query::gov::{get_tally_v1beta1};
use cosmos_rust_package::api::custom::types::gov::proposal_ext::{ProposalExt, ProposalStatus};
use log::info;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;

use crate::plugin::interface::agent::governance::proposals::api::get_proposals_by;
use cosmos_rust_package::api::custom::types::TallyResultType;
use std::pin::Pin;

use crate::plugin::interface::agent::governance::GOVERNANCE_PREFIX;

pub const TALLY_RESULT_PREFIX: &str = "tally_result_";

#[derive(Clone)]
pub struct TallyResultsAgent {
    pub continue_at_key_prefix: String,
    pub update_interval_in_secs: i64,
    pub retry_delay_in_secs: HashMap<TallyResultsTasks,i64>,
    initial_retry_delay: i64,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct TallyResultsTasks {
    blockchain_name: String,
    proposal_status: ProposalStatus,
}

impl Default for TallyResultsAgent {
    fn default() -> Self {
        Self {
            continue_at_key_prefix: "fetch_tally_results_for".to_string(),
            update_interval_in_secs: 60 * 15,
            retry_delay_in_secs: HashMap::new(),
            initial_retry_delay: 60,
        }
    }
}

impl TallyResultsAgent {
    async fn try_fetch_tally_results(
        agent: TallyResultsAgent,
        blockchain: SupportedBlockchain,
        proposal_status: ProposalStatus,
    ) -> anyhow::Result<()> {
        info!("now running: {:?}", (&blockchain.name, &proposal_status));

        let task_store = AGENT_STORE.clone();

        let key = format!("{}_{}", blockchain.name, proposal_status.to_string());
        let continue_at_key = format!("{}_{}", agent.continue_at_key_prefix, key);

        let next_index = get_next_index(&continue_at_key);
        let mut values = get_proposals_by(
            Some(vec![blockchain.clone()]),
            Some(vec![proposal_status.clone()]),
            next_index,
        );
        values.sort_by_key(|k| k.get_proposal_id());

        for each in values {
            let id = each.get_proposal_id();
            let result = get_tally_v1beta1(blockchain.clone(), id).await;
            match result {
                Ok(_) => {
                    set_next_index(&continue_at_key, None)?;
                }
                Err(_) => {
                    set_next_index(&continue_at_key, Some(id))?;
                }
            };

            let mut output: Result<(), anyhow::Error> = Ok(());
            let data: Result<TallyResultType, EntryError> = match result {
                Ok(params) => Ok(params),
                Err(err) => {
                    output = Err(err);
                    Err(EntryError::Error(
                        output.as_ref().err().unwrap().to_string(),
                    ))
                }
            };

            let key1 = get_tally_result_entry_key(&each);
            task_store.insert_if_not_exists(&key1, data)?;

            output?;
        }
        Ok(())
    }

    pub fn fetch_tally_results(
        &self,
        blockchain: SupportedBlockchain,
        proposal_status: ProposalStatus,
    ) -> Pin<Box<dyn Future<Output = TaskResult<TallyResultsTasks>> + Send>> {
        let agent = self.clone();

        Box::pin(async move {
            TaskResult::new(
                TallyResultsTasks {
                    blockchain_name: blockchain.name.to_owned(),
                    proposal_status: proposal_status.clone(),
                },
                TallyResultsAgent::try_fetch_tally_results(agent, blockchain, proposal_status)
                    .await,
            )
        })
    }
}

impl Agent for TallyResultsAgent {
    type TaskType = TallyResultsTasks;

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
                for proposal_status in vec![ProposalStatus::StatusVotingPeriod] {
                    let task_type = TallyResultsTasks {
                        blockchain_name: value.name.to_owned(),
                        proposal_status: proposal_status.clone(),
                    };
                    if !tasks_pending.iter().any(|x| x == &task_type) {
                        fns.insert(
                            task_type,
                            self_clone.fetch_tally_results(value.clone(), proposal_status),
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

fn get_tally_result_entry_key(proposal: &ProposalExt) -> String {
    let key = format!(
        "{}{}{}{}",
        GOVERNANCE_PREFIX,
        TALLY_RESULT_PREFIX,
        proposal.blockchain.name,
        proposal.get_proposal_id()
    );
    key
}

pub fn try_get_tally_result(proposal: &ProposalExt) -> Option<TallyResultType> {
    let task_store = AGENT_STORE.clone();

    let key = get_tally_result_entry_key(proposal);

    match task_store.get::<TallyResultType>(&key, &RetrievalMethod::GetOk) {
        Ok(entry) => entry.data.ok(),
        Err(_) => None,
    }
}
