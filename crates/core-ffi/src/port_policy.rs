//! Unprivileged Ports draft/agent adapters over the same policy used by Core.
use crate::{CoreError, CorePortPolicy, CorePortRule};
use vpnhide_core_rpc::{canonical, port_policy as policy};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CorePortPolicyPreset {
    pub id: String,
    pub rules: Vec<CorePortRule>,
}
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CorePortRuleIntent {
    pub protocol: String,
    pub start: i64,
    pub end: i64,
}
#[uniffi::export]
pub fn port_policy_presets() -> Vec<CorePortPolicyPreset> {
    policy::presets()
        .into_iter()
        .map(|p| CorePortPolicyPreset {
            id: p.id,
            rules: p.rules.into_iter().map(Into::into).collect(),
        })
        .collect()
}
#[uniffi::export]
pub fn port_policy_for_preset(id: String) -> Option<CorePortPolicy> {
    policy::preset_policy(&id).map(Into::into)
}
#[uniffi::export]
pub fn normalized_port_rules(rules: Vec<CorePortRule>) -> Vec<CorePortRule> {
    let mut rules = rules.into_iter().map(Into::into).collect();
    policy::normalize_rules(&mut rules);
    rules.into_iter().map(Into::into).collect()
}
#[uniffi::export]
pub fn normalized_port_policy(policy: Option<CorePortPolicy>) -> Option<CorePortPolicy> {
    policy.map(|p| {
        let mut p = p.into();
        canonical::normalize_port_policy(&mut p);
        p.into()
    })
}
#[uniffi::export]
pub fn parse_agent_port_policy(
    mode: String,
    preset: Option<String>,
    rules: Vec<CorePortRuleIntent>,
) -> Result<Option<CorePortPolicy>, CoreError> {
    let rules = rules
        .into_iter()
        .map(|r| policy::PortRuleIntent {
            protocol: r.protocol,
            start: r.start,
            end: r.end,
        })
        .collect::<Vec<_>>();
    policy::parse_agent_intent(&mode, preset.as_deref(), &rules)
        .map(|p| p.map(Into::into))
        .map_err(|detail| CoreError::InvalidInput { detail })
}
