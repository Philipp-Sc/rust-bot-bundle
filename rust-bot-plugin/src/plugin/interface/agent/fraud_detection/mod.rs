use log::info;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;

use crate::plugin::interface::{Agent, TaskResult};

use cosmos_rust_package::api::custom::types::gov::proposal_ext::{ProposalExt, ProposalStatus};

use rust_bert_fraud_detection_socket_ipc::ipc::client_send_rust_bert_fraud_detection_request;

use crate::plugin::interface::agent::governance::proposals::fetch::PROPOSAL_PREFIX;
use crate::plugin::interface::agent::governance::GOVERNANCE_PREFIX;
use crate::plugin::interface::agent::AGENT_STORE;
use crate::plugin::store::fallback_entry_store::{Entry, EntryError, RetrievalMethod};
use cosmos_rust_package::api::core::cosmos::channels::SupportedBlockchain;
use serde::{Deserialize, Serialize};
use sled::Event;

pub const FRAUD_DETECTION_PREFIX: &str = "fraud_detection_";

#[derive(Clone)]
pub struct FraudDetectionAgent {
    pub unix_socket: Option<String>,
    pub csv_file: Option<String>,
    pub update_interval_in_secs: i64,
    pub retry_delay_in_secs: HashMap<FraudDetectionTasks,i64>,
    initial_retry_delay: i64,
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub enum FraudDetectionTasks {
    FetchFraudDetection,
}

impl Default for FraudDetectionAgent {
    fn default() -> Self {
        Self {
            unix_socket: Some("./tmp/rust_bert_fraud_detection_socket".to_string()),
            csv_file: Some("./tmp/governance_proposal_spam_likelihood.csv".to_string()),
            update_interval_in_secs: 0,
            retry_delay_in_secs: HashMap::new(),
            initial_retry_delay: 0,
        }
    }
}

impl FraudDetectionAgent {
    async fn try_fetch_fraud_detection(agent: FraudDetectionAgent) -> anyhow::Result<()> {
        info!("now running");

        let prefix = format!("{}{}", GOVERNANCE_PREFIX, PROPOSAL_PREFIX);

        let historic_proposal_statuses = vec![
            ProposalStatus::StatusPassed,
            ProposalStatus::StatusFailed,
            ProposalStatus::StatusRejected,
            ProposalStatus::StatusNil,
        ];

        let active_proposal_statuses = vec![
            ProposalStatus::StatusDepositPeriod,
            ProposalStatus::StatusVotingPeriod,
        ];

        loop {
            let mut subscriber = AGENT_STORE.watch_prefix(&mut prefix.as_bytes().to_vec());
            while let Some(event) = subscriber.next() {
                match event {
                    Event::Insert { key, value } => {
                        let _key = String::from_utf8_lossy(&key);

                        if let Some::<Entry<ProposalExt>>(Entry {
                            data: Ok(proposal), ..
                        }) = value.to_vec().try_into().ok()
                        {
                            if let Some(csv_file) = &agent.csv_file {
                                if historic_proposal_statuses.contains(&proposal.status) {
                                    let text = format!(
                                        "{}\n\n{}",
                                        proposal.get_title(),
                                        proposal.get_description()
                                    );

                                    if let Some(value) = proposal.spam_likelihood() {
                                        // Check if the file already exists
                                        let file_exists = std::path::Path::new(csv_file).exists();

                                        let file = OpenOptions::new()
                                            .write(true)
                                            .create(true)
                                            .append(true)
                                            .open(csv_file)
                                            .expect("Failed to open the CSV file in append mode");

                                        let mut wtr = csv::Writer::from_writer(file);

                                        // Write the header record if the file is empty (only needed in the first iteration)
                                        if !file_exists {
                                            wtr.write_record(&["body", "label"]).unwrap();
                                        }
                                        wtr.write_record(&[
                                            text.as_str(),
                                            value.to_string().as_str(),
                                        ])
                                        .unwrap();

                                        // Flush the writer to ensure all records are written to the file
                                        wtr.flush().unwrap();
                                    }
                                }
                            }
                            if let Some(unix_socket) = &agent.unix_socket {
                                if active_proposal_statuses.contains(&proposal.status) {
                                    let task_store = AGENT_STORE.clone();

                                    let fraud_classification_entry_key =
                                        get_fraud_classification_entry_key(&proposal);

                                    if !task_store.contains_key(&fraud_classification_entry_key) {
                                        let text = get_relevant_text(&proposal);

                                        let result = client_send_rust_bert_fraud_detection_request(
                                            &unix_socket,
                                            vec![text.clone()],
                                        );

                                        let data = match result {
                                            Ok(rust_bert_fraud_detection) => {
                                                let fraud_classification =
                                                    GovernanceProposalFraudClassification {
                                                        blockchain: proposal.blockchain.clone(),
                                                        proposal_id: proposal.get_proposal_id(),
                                                        text,
                                                        fraud_prediction: rust_bert_fraud_detection
                                                            .fraud_probabilities[0],
                                                    };
                                                Ok(fraud_classification)
                                            }
                                            Err(err) => Err(EntryError::Error(err.to_string())),
                                        };

                                        task_store
                                            .insert::<GovernanceProposalFraudClassificationType>(
                                                &fraud_classification_entry_key,
                                                data,
                                            )?;
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

    fn fetch_fraud_detection(
        &self,
    ) -> Pin<Box<dyn Future<Output = TaskResult<FraudDetectionTasks>> + Send>> {
        let agent = self.clone();

        Box::pin(async move {
            TaskResult::new(
                FraudDetectionTasks::FetchFraudDetection,
                FraudDetectionAgent::try_fetch_fraud_detection(agent).await,
            )
        })
    }
}

impl Agent for FraudDetectionAgent {
    type TaskType = FraudDetectionTasks;

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
            .any(|x| x == &FraudDetectionTasks::FetchFraudDetection)
        {
            fns.insert(
                FraudDetectionTasks::FetchFraudDetection,
                self_clone.fetch_fraud_detection(),
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

pub type GovernanceProposalFraudClassificationType = GovernanceProposalFraudClassification;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct GovernanceProposalFraudClassification {
    pub blockchain: SupportedBlockchain,
    pub proposal_id: u64,
    pub text: String,
    pub fraud_prediction: f64,
}

fn get_relevant_text(proposal: &ProposalExt) -> String {
    let title = proposal.get_title();
    let description = proposal.get_description();
    let text = format!("{}\n\n{}", &title, &description);
    text
}

pub fn get_relevant_text_as_hash(proposal: &ProposalExt) -> u64 {
    let mut s = DefaultHasher::new();
    get_relevant_text(proposal).hash(&mut s);
    s.finish()
}

fn get_fraud_classification_entry_key(proposal: &ProposalExt) -> String {
    let key = format!(
        "{}{}{}_{}_{}",
        GOVERNANCE_PREFIX,
        FRAUD_DETECTION_PREFIX,
        proposal.blockchain.name,
        proposal.get_proposal_id(),
        get_relevant_text_as_hash(proposal)
    );
    key
}

pub fn try_get_fraud_classification(
    proposal: &ProposalExt,
) -> Option<GovernanceProposalFraudClassificationType> {
    let task_store = AGENT_STORE.clone();

    let key = get_fraud_classification_entry_key(proposal);

    match task_store.get::<GovernanceProposalFraudClassificationType>(&key, &RetrievalMethod::GetOk)
    {
        Ok(entry) => entry.data.ok(),
        Err(_) => None,
    }
}
