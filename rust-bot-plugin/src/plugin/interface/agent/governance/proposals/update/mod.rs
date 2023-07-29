use crate::plugin::interface::{Agent, TaskResult};

use log::{error, info};

use std::collections::{HashMap, HashSet};
use std::fs;
use std::future::Future;
use std::hash::Hash;

use crate::plugin::interface::agent::AGENT_STORE;
use sled::Event;
use std::pin::Pin;

use crate::plugin::interface::agent::fraud_detection::{
    GovernanceProposalFraudClassification, FRAUD_DETECTION_PREFIX,
};
use crate::plugin::interface::agent::governance::params::PARAMS_PREFIX;
use crate::plugin::interface::agent::governance::proposals::api::{
    get_proposal_view_by, get_proposal_views_by, proposal_to_view, GovernanceProposalView,
};
use crate::plugin::interface::agent::governance::proposals::fetch::PROPOSAL_PREFIX;
use crate::plugin::interface::agent::governance::tally_results::TALLY_RESULT_PREFIX;
use crate::plugin::interface::agent::governance::validators::VALIDATOR_PREFIX;
use crate::plugin::interface::agent::governance::GOVERNANCE_PREFIX;
use crate::plugin::interface::agent::staking::pool::POOL_PREFIX;
use crate::plugin::store::fallback_entry_store::Entry;
use cosmos_rust_package::api::custom::types::gov::params_ext::ParamsExt;
use cosmos_rust_package::api::custom::types::gov::proposal_ext::ProposalExt;
use cosmos_rust_package::api::custom::types::gov::tally_ext::TallyResultExt;
use cosmos_rust_package::api::custom::types::staking::pool_ext::PoolExt;
use cosmos_rust_package::api::custom::types::staking::validators_ext::ValidatorsExt;

// watch one prefix that is prefix of all governance agents, inserts.
// handle each type .ie. identify which proposal view needs to be updated.

// that way we have a direct notification path, without check loops.

// we can then simply listen for governance proposal views update events, and can then send the notification.

#[derive(Clone)]
pub struct GovernanceProposalViewAgent {
    pub continue_at_key_prefix: String,
    pub rate_limit_delay_in_secs: u64,
    pub update_interval_in_secs: i64,
    pub retry_delay_in_secs: HashMap<GovernanceProposalViewTasks,i64>,
    initial_retry_delay: i64,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub enum GovernanceProposalViewTasks {
    Update,
}

impl Default for GovernanceProposalViewAgent {
    fn default() -> Self {
        Self {
            continue_at_key_prefix: "update_proposal_views_for".to_string(),
            rate_limit_delay_in_secs: 10u64,
            update_interval_in_secs: 0,
            retry_delay_in_secs: HashMap::new(),
            initial_retry_delay: 0,
        }
    }
}

impl GovernanceProposalViewAgent {
    async fn try_update_proposal_views(_agent: GovernanceProposalViewAgent) -> anyhow::Result<()> {
        info!("now running: try_update_proposal_views");

        loop {
            let mut subscriber =
                AGENT_STORE.watch_prefix(&mut GOVERNANCE_PREFIX.as_bytes().to_vec());
            while let Some(event) = subscriber.next() {
                match event {
                    Event::Insert { key, value } => {
                        let key = String::from_utf8_lossy(&key);
                        info!("Event: Updated value for key: {}", key);

                        let mut proposal_views = Vec::new();

                        if key.contains(PROPOSAL_PREFIX) {
                            if let Some::<Entry<ProposalExt>>(Entry {
                                data: Ok(proposal), ..
                            }) = value.to_vec().try_into().ok()
                            {
                                proposal_views.push(proposal_to_view(proposal));
                            }
                        } else if key.contains(FRAUD_DETECTION_PREFIX) {
                            if let Some::<Entry<GovernanceProposalFraudClassification>>(Entry {
                                data: Ok(fraud_classification),
                                ..
                            }) = value.to_vec().try_into().ok()
                            {
                                if let Some(proposal_view) = get_proposal_view_by(
                                    &fraud_classification.blockchain,
                                    fraud_classification.proposal_id,
                                ) {
                                    proposal_views.push(proposal_view);
                                }
                            }
                        } else if key.contains(TALLY_RESULT_PREFIX) {
                            if let Some::<Entry<TallyResultExt>>(Entry {
                                data: Ok(tally_result),
                                ..
                            }) = value.to_vec().try_into().ok()
                            {
                                if let Some(proposal_view) = get_proposal_view_by(
                                    &tally_result.blockchain,
                                    tally_result.proposal_id,
                                ) {
                                    proposal_views.push(proposal_view);
                                }
                            }
                        } else if key.contains(PARAMS_PREFIX) {
                            if let Some::<Entry<ParamsExt>>(Entry {
                                data: Ok(params), ..
                            }) = value.to_vec().try_into().ok()
                            {
                                proposal_views.append(&mut get_proposal_views_by(
                                    Some(vec![params.blockchain]),
                                    None,
                                ));
                            }
                        } else if key.contains(VALIDATOR_PREFIX) {
                            if let Some::<Entry<ValidatorsExt>>(Entry {
                                data: Ok(validator),
                                ..
                            }) = value.to_vec().try_into().ok()
                            {
                                proposal_views.append(&mut get_proposal_views_by(
                                    Some(vec![validator.blockchain]),
                                    None,
                                ));
                            }
                        } else if key.contains(POOL_PREFIX) {
                            if let Some::<Entry<PoolExt>>(Entry { data: Ok(pool), .. }) =
                                value.to_vec().try_into().ok()
                            {
                                proposal_views.append(&mut get_proposal_views_by(
                                    Some(vec![pool.blockchain]),
                                    None,
                                ));
                            }
                        }

                        if !proposal_views.is_empty() {
                            let task_store = AGENT_STORE.clone();

                            for proposal_view in proposal_views {
                                let html_content = proposal_view.generate_html();
                                let file_path = format!(
                                    "tmp/governance_proposals/{}/{}.html",
                                    proposal_view.proposal_blockchain_name,
                                    proposal_view.proposal_id
                                );

                                // Extract the directory path from the file_path
                                let dir_path = match std::path::Path::new(&file_path).parent() {
                                    Some(path) => path,
                                    None => {
                                        error!("Invalid file_path: {}", file_path);
                                        panic!();
                                    }
                                };

                                // Create the directories if they do not exist
                                match fs::create_dir_all(dir_path) {
                                    Ok(()) => {
                                        // Directories are created or already exist, proceed with writing the file
                                        match fs::write(&file_path, &html_content) {
                                            Ok(_) => info!("HTML content written to {}", file_path),
                                            Err(e) => error!(
                                                "Failed to write HTML content to file: {}",
                                                e
                                            ),
                                        }
                                    }
                                    Err(e) => {
                                        error!(
                                            "Failed to create directories for file {}: {}",
                                            file_path, e
                                        );
                                        panic!();
                                    }
                                }

                                let key = get_proposal_view_entry_key(&proposal_view);
                                match task_store.insert_if_not_exists::<GovernanceProposalView>(
                                    &key,
                                    Ok(proposal_view),
                                ) {
                                    Ok(_) => {}
                                    Err(err) => {
                                        error!(
                                            "Failed to insert key: {}, {}",
                                            key,
                                            err.to_string()
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Event::Remove { key: _ } => {}
                }
            }
        }
    }

    pub fn update_proposal_views(
        &self,
    ) -> Pin<Box<dyn Future<Output = TaskResult<GovernanceProposalViewTasks>> + Send>> {
        let agent = self.clone();

        Box::pin(async move {
            TaskResult::new(
                GovernanceProposalViewTasks::Update,
                GovernanceProposalViewAgent::try_update_proposal_views(agent).await,
            )
        })
    }
}

impl Agent for GovernanceProposalViewAgent {
    type TaskType = GovernanceProposalViewTasks;

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
            .any(|x| x == &GovernanceProposalViewTasks::Update)
        {
            fns.insert(
                GovernanceProposalViewTasks::Update,
                self_clone.update_proposal_views(),
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

pub fn get_proposal_view_entry_key(proposal_view: &GovernanceProposalView) -> String {
    let key = format!(
        "{}{}{}_{}",
        GOVERNANCE_PREFIX,
        PROPOSAL_PREFIX,
        proposal_view.proposal_blockchain_name,
        proposal_view.proposal_id
    );
    key
}
