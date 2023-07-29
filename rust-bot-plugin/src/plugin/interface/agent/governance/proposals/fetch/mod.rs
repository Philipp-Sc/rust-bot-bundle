use crate::plugin::interface::agent::chain_registry::try_get_chain_registry;
use crate::plugin::interface::agent::{get_next_key, set_next_key, AGENT_STORE};
use crate::plugin::interface::{Agent, TaskResult};

use cosmos_rust_package::api::core::cosmos::channels::SupportedBlockchain;
use cosmos_rust_package::api::custom::types::gov::proposal_ext::{ProposalExt, ProposalStatus};
use log::info;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::Hash;

use std::pin::Pin;
use cosmos_rust_package::api::custom::query::gov::{get_proposals};

use crate::plugin::interface::agent::governance::GOVERNANCE_PREFIX;
use strum::IntoEnumIterator;

pub const PROPOSAL_PREFIX: &str = "proposal_";

#[derive(Clone)]
pub struct GovernanceProposalFetchAgent {
    pub continue_at_key_prefix: String,
    pub update_interval_in_secs: i64,
    pub retry_delay_in_secs: HashMap<GovernanceProposalFetchTask,i64>,
    initial_retry_delay: i64,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct GovernanceProposalFetchTask {
    blockchain_name: String,
    proposal_status: ProposalStatus,
}

impl Default for GovernanceProposalFetchAgent {
    fn default() -> Self {
        Self {
            continue_at_key_prefix: "fetch_proposals_for".to_string(),
            update_interval_in_secs: 60 * 5,
            retry_delay_in_secs: HashMap::new(),
            initial_retry_delay: 60,
        }
    }
}

impl GovernanceProposalFetchAgent {
    async fn try_fetch_proposals(
        agent: GovernanceProposalFetchAgent,
        blockchain: SupportedBlockchain,
        proposal_status: ProposalStatus,
    ) -> anyhow::Result<()> {
        info!("now running: {:?}", (&blockchain.name, &proposal_status));

        let task_store = AGENT_STORE.clone();

        let continue_at_key = format!(
            "{}_{}_{}",
            agent.continue_at_key_prefix,
            blockchain.name,
            proposal_status.to_string()
        );

        let mut next_key = get_next_key(&continue_at_key);

        let result = get_proposals(
            blockchain.clone(),
            proposal_status.clone(),
            next_key.clone(),
        )
        .await;

        if let Ok(proposals) = result {
            for proposal in proposals.1 {
                let proposal_key = get_proposal_entry_key(&blockchain, proposal.get_proposal_id());
                task_store.insert_if_not_exists::<ProposalExt>(&proposal_key, Ok(proposal))?;
            }

            next_key = proposals.0.clone();

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

    pub fn fetch_proposals(
        &self,
        blockchain: SupportedBlockchain,
        proposal_status: ProposalStatus,
    ) -> Pin<Box<dyn Future<Output = TaskResult<GovernanceProposalFetchTask>> + Send>> {
        let agent = self.clone();

        Box::pin(async move {
            TaskResult::new(
                GovernanceProposalFetchTask {
                    blockchain_name: blockchain.name.to_owned(),
                    proposal_status: proposal_status.clone(),
                },
                GovernanceProposalFetchAgent::try_fetch_proposals(
                    agent,
                    blockchain,
                    proposal_status,
                )
                .await,
            )
        })
    }
}

impl Agent for GovernanceProposalFetchAgent {
    type TaskType = GovernanceProposalFetchTask;

    fn get_tasks(
        &self,
        tasks_pending: HashSet<Self::TaskType>,
    ) -> HashMap<
                Self::TaskType,
                Pin<Box<dyn Future<Output = TaskResult<Self::TaskType>> + Send>>
            >
        {
        let self_clone = self.clone();

        let mut fns = HashMap::new();

        try_get_chain_registry().map(|x| {
            for (_, value) in x.into_iter() {
                for proposal_status in ProposalStatus::iter() {
                    let task_type = GovernanceProposalFetchTask {
                        blockchain_name: value.name.to_owned(),
                        proposal_status: proposal_status.clone(),
                    };
                    if !tasks_pending.iter().any(|x| x == &task_type) {
                        fns.insert(
                            task_type,
                            self_clone.fetch_proposals(value.clone(), proposal_status),
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

pub fn get_proposal_entry_key(blockchain: &SupportedBlockchain, proposal_id: u64) -> String {
    let key = format!(
        "{}{}{}_{}",
        GOVERNANCE_PREFIX, PROPOSAL_PREFIX, blockchain.name, proposal_id
    );
    key
}
