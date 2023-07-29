use crate::plugin::interface::agent::fraud_detection::{
    try_get_fraud_classification, GovernanceProposalFraudClassification,
};
use crate::plugin::interface::agent::governance::params::try_get_params;
use crate::plugin::interface::agent::governance::tally_results::try_get_tally_result;
use crate::plugin::interface::agent::staking::pool::try_get_pool;
use crate::plugin::interface::agent::AGENT_STORE;
use crate::plugin::store::fallback_entry_store::{Entry, RetrievalMethod};
use chrono::{DateTime, Utc};
use cosmos_rust_package::api::core::cosmos::channels::SupportedBlockchain;
use cosmos_rust_package::api::custom::types::gov::params_ext::ParamsExt;
use cosmos_rust_package::api::custom::types::gov::proposal_ext::ProposalExt;
use cosmos_rust_package::api::custom::types::gov::proposal_ext::ProposalStatus;
use cosmos_rust_package::api::custom::types::gov::tally_v1beta1_ext::TallyResultV1Beta1Ext;
use cosmos_rust_package::api::custom::types::staking::pool_ext::PoolExt;
use cosmos_rust_package::api::custom::types::GovernanceProposalsType;
use serde::{Deserialize, Serialize};

use std::cmp::PartialEq;

use minify_html::{minify, Cfg};

use crate::plugin::interface::agent::governance::proposals::fetch::{
    get_proposal_entry_key, PROPOSAL_PREFIX,
};
use crate::plugin::interface::agent::governance::GOVERNANCE_PREFIX;
use askama::Template;

pub fn get_proposal_by(blockchain: &SupportedBlockchain, proposal_id: u64) -> Option<ProposalExt> {
    let task_store = AGENT_STORE.clone();
    let key = get_proposal_entry_key(blockchain, proposal_id);
    match task_store.get::<ProposalExt>(&key, &RetrievalMethod::GetOk) {
        Ok(Entry {
            data: Ok(proposal), ..
        }) => Some(proposal),
        _ => None,
    }
}

pub fn get_proposal_view_by(
    blockchain: &SupportedBlockchain,
    proposal_id: u64,
) -> Option<GovernanceProposalView> {
    let task_store = AGENT_STORE.clone();
    let key = get_proposal_entry_key(blockchain, proposal_id);
    match task_store.get::<ProposalExt>(&key, &RetrievalMethod::GetOk) {
        Ok(Entry {
            data: Ok(proposal), ..
        }) => Some(proposal_to_view(proposal)),
        _ => None,
    }
}

pub fn proposal_to_view(proposal: ProposalExt) -> GovernanceProposalView {
    let tally_result = try_get_tally_result(&proposal);
    let blockchain_pool = try_get_pool(&proposal.blockchain);
    let deposit_param = try_get_params(&proposal.blockchain, "deposit");
    let voting_param = try_get_params(&proposal.blockchain, "voting");
    let tallying_param = try_get_params(&proposal.blockchain, "tallying");

    let fraud_classification = try_get_fraud_classification(&proposal);

    GovernanceProposalView::new(
        &proposal,
        &fraud_classification,
        tally_result,
        tallying_param,
        deposit_param,
        voting_param,
        blockchain_pool,
    )
}

pub fn get_proposals_by(
    blockchains: Option<Vec<SupportedBlockchain>>,
    proposal_statuses: Option<Vec<ProposalStatus>>,
    next_index: Option<u64>,
) -> Vec<ProposalExt> {
    let task_store = AGENT_STORE.clone();

    let mut values: GovernanceProposalsType = Vec::new();

    let key_prefix = format!("{}{}", GOVERNANCE_PREFIX, PROPOSAL_PREFIX);

    for (_val_key, val) in
        task_store.value_iter::<ProposalExt>(Some(&key_prefix), &RetrievalMethod::GetOk)
    {
        match val {
            Entry {
                data: Ok(proposal),
                timestamp: _,
            } => {
                if (proposal_statuses.is_none()
                    || proposal_statuses
                        .as_ref()
                        .unwrap()
                        .iter()
                        .any(|status| *status == proposal.status))
                    && (blockchains.is_none()
                        || blockchains
                            .as_ref()
                            .unwrap()
                            .iter()
                            .any(|b| b.name == proposal.blockchain.name))
                    && (next_index.is_none() || proposal.get_proposal_id() >= next_index.unwrap())
                {
                    values.push(proposal);
                }
            }
            _ => {}
        }
    }
    values
}

pub fn get_proposal_views_by(
    blockchains: Option<Vec<SupportedBlockchain>>,
    proposal_statuses: Option<Vec<ProposalStatus>>,
) -> Vec<GovernanceProposalView> {
    get_proposals_by(blockchains, proposal_statuses, None)
        .into_iter()
        .map(|proposal| proposal_to_view(proposal))
        .collect()
}

#[derive(Template, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[template(path = "governance_proposal_template.html")]
pub struct GovernanceProposalView {
    pub proposal_id: String,
    pub proposal_blockchain_name: String,
    proposal_blockchain: String,
    proposal_type: String,
    proposal_title: String,
    proposal_description: String,
    proposal_link: String,
    proposal_status_icon: String,
    proposal_deposit_param: String,
    proposal_voting_param: String,
    proposal_tallying_param: String,
    proposal_tally_result: Option<String>,
    proposal_tally_result_detail: Option<String>,
    proposal_voter_turnout: Option<String>,
    proposal_blockchain_pool_details: Option<String>,
    proposal_state: String,
    proposal_state_detail: Option<String>,
    last_updated: String,
    proposal_submitted: String,
    website_language_label: String,
    website_show_more_button: String,
    website_open_in_button: String,
    website_deposit_param_label: String,
    website_voting_param_label: String,
    website_tallying_param_label: String,
    website_footer: String,
    js_const_fraud_warning: String,
    js_const_fraud_alert: String,
    js_const_high_veto_alert: String,
    js_const_deposit_period_warning: String,
    js_const_fraud_risk: String,
    js_const_spam_likelihood: String,
    js_const_in_deposit_period: String,
}

impl GovernanceProposalView {
    pub fn new(
        proposal: &ProposalExt,
        fraud_classification: &Option<GovernanceProposalFraudClassification>,
        tally_result: Option<TallyResultV1Beta1Ext>,
        tallying_param: Option<ParamsExt>,
        deposit_param: Option<ParamsExt>,
        voting_param: Option<ParamsExt>,
        blockchain_pool: Option<PoolExt>,
    ) -> Self {
        Self {
            proposal_id: proposal.get_proposal_id().to_string(),
            proposal_blockchain_name: proposal.blockchain.name.to_string(),
            proposal_blockchain: proposal.blockchain.display.to_string(),
            proposal_type: proposal.messages_as_proposal_content().iter().map(|x| x.to_string()).collect::<Vec<String>>().join("\n"),
            proposal_title: proposal.get_title(),
            proposal_description: proposal.get_description(),
            proposal_link: proposal.governance_proposal_link(),
            proposal_status_icon: proposal.status.to_icon(),
            proposal_deposit_param: deposit_param.as_ref().map(|value| format!("{}", value)).unwrap_or("The deposit parameters have not been fetched yet.\nPlease refresh the page to try again.".to_string()),
            proposal_voting_param: voting_param.as_ref().map(|value| format!("{}", value)).unwrap_or("The voting parameters have not been fetched yet.\nPlease refresh the page to try again.".to_string()),
            proposal_tallying_param: tallying_param.as_ref().map(|value| format!("{}", value)).unwrap_or("The tallying parameters have not been fetched yet.\nPlease refresh the page to try again.".to_string()),
            proposal_tally_result: tally_result.as_ref().map(|value| format!("{}", value.current_tally())),
            proposal_tally_result_detail: tally_result.as_ref().map(|value| format!("{}", value.tally_details())),
            proposal_voter_turnout: blockchain_pool.as_ref().map(|pool_ext| {
                pool_ext.get_voter_turnout(proposal.total_votes().or(tally_result.as_ref().map(|x| x.total_votes()).flatten()))
            }).flatten().map(|value| format!("👥 {}", value)),
            proposal_blockchain_pool_details: blockchain_pool.as_ref().map(|pool_ext| pool_ext.get_pool_details()).flatten(),
            proposal_state: proposal.proposal_state(),
            proposal_state_detail: proposal.tally_details(),
            last_updated: {
                let now: DateTime<Utc> = Utc::now();
                let timestamp = now.to_rfc2822().replace("+0000", "UTC");
                format!("Last updated: {}", timestamp)
            },
            proposal_submitted: format!("Submitted: {}", proposal.proposal_submitted()),
            website_language_label: "Language:".to_string(),
            website_show_more_button: "Show More".to_string(),
            website_open_in_button: "Open in 🛰️/🅺".to_string(),
            website_deposit_param_label: "⚙️ Deposit Parameters".to_string(),
            website_voting_param_label: "⚙️ Voting Parameters".to_string(),
            website_tallying_param_label: "⚙️ Tallying Parameters".to_string(),
            website_footer: "This website was created by <a href=\"https://github.com/Philipp-Sc/cosmos-rust-bot/tree/development/workspace/cosmos-rust-bot#readme\">CosmosRustBot</a>.</br>Give <a href=\"https://github.com/Philipp-Sc/cosmos-rust-bot/issues\">Feedback</a>.".to_string(),
            js_const_fraud_warning: "⚠ WARNING: Moderate fraud risk. Stay safe! ⚠".to_string(),
            js_const_fraud_alert: "🚨 ALERT: High fraud risk. Remember, if it seems too good to be true, it probably is. 🚨".to_string(),
            js_const_high_veto_alert: "🚨 ALERT: High fraud risk. High percentage of NoWithVeto votes! 🚨".to_string(),
            js_const_deposit_period_warning: "⚠ CAUTION: Fraud risk during deposit period. ⚠".to_string(),
            js_const_fraud_risk: fraud_classification.as_ref().map(|x| x.fraud_prediction).unwrap_or(0.0).to_string(),
            js_const_spam_likelihood: proposal.spam_likelihood().unwrap_or(tally_result.as_ref().map(|x| x.spam_likelihood()).flatten().unwrap_or(0f64)).to_string(),
            js_const_in_deposit_period: proposal.is_in_deposit_period().to_string(),
        }
    }

    pub fn generate_html(&self) -> String {
        let mut cfg = Cfg::spec_compliant();
        cfg.minify_css = true;
        cfg.minify_js = true;

        let html_output = self.render().unwrap();

        let minified = minify(html_output.as_bytes(), &cfg);
        String::from_utf8(minified.to_vec()).unwrap()
    }
}
