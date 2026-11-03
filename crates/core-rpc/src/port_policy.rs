//! Portable Ports intent/catalog/rule owner. Disk readers retain their separate
//! tolerant/strict admission behavior; neither reading a preset id nor normalizing
//! rules replaces the explicit ranges stored in an existing configuration.
use crate::{PortPolicy, PortProtocol, PortRule};

pub const COMMON_PROXY_PRESET: &str = "common_proxy";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortPolicyPreset {
    pub id: String,
    pub rules: Vec<PortRule>,
}

pub fn presets() -> Vec<PortPolicyPreset> {
    vec![PortPolicyPreset {
        id: COMMON_PROXY_PRESET.into(),
        rules: [
            (1080, 1080),
            (7890, 7892),
            (8080, 8080),
            (8888, 8888),
            (9050, 9050),
            (9090, 9090),
            (9150, 9150),
            (10808, 10808),
        ]
        .into_iter()
        .map(|(start, end)| PortRule {
            protocol: PortProtocol::Both,
            start,
            end,
        })
        .collect(),
    }]
}

pub fn preset_policy(id: &str) -> Option<PortPolicy> {
    presets()
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| PortPolicy {
            mode: "preset".into(),
            preset: Some(p.id),
            rules: p.rules,
        })
}

pub fn normalize_rules(rules: &mut Vec<PortRule>) {
    rules.sort_by_key(|rule| {
        (
            rule.start,
            rule.end,
            match rule.protocol {
                PortProtocol::Both => 0,
                PortProtocol::Tcp => 1,
                PortProtocol::Udp => 2,
            },
        )
    });
    rules.dedup();
}

/// Strings here are stable existing adapter errors, not privileged outcomes.
pub fn rule(protocol: &str, start: i64, end: i64) -> Result<PortRule, String> {
    let start = u16::try_from(start)
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| "Port start must be 1..65535".to_owned())?;
    let end = u16::try_from(end)
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| "Port end must be 1..65535".to_owned())?;
    if start > end {
        return Err("Port start must not be greater than end".into());
    }
    Ok(PortRule {
        protocol: match protocol {
            "tcp" => PortProtocol::Tcp,
            "udp" => PortProtocol::Udp,
            _ => PortProtocol::Both,
        },
        start,
        end,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRuleIntent {
    pub protocol: String,
    pub start: i64,
    pub end: i64,
}

/// Existing agent semantics: All/Preset ignore supplied custom rules, unknown
/// mode labels use Custom, and unknown protocol labels use Both. This parser
/// produces a preview; Core independently validates the committed canonical.
pub fn parse_agent_intent(
    mode: &str,
    preset: Option<&str>,
    rules: &[PortRuleIntent],
) -> Result<Option<PortPolicy>, String> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "all" => Ok(None),
        "preset" => {
            let id = preset.ok_or_else(|| "Port preset is required".to_owned())?;
            preset_policy(id)
                .map(Some)
                .ok_or_else(|| format!("Unknown port preset: {id}"))
        }
        _ => {
            if rules.is_empty() {
                return Err("Custom port policy requires at least one rule".into());
            }
            let mut rules = rules
                .iter()
                .map(|r| rule(&r.protocol.trim().to_ascii_lowercase(), r.start, r.end))
                .collect::<Result<Vec<_>, _>>()?;
            normalize_rules(&mut rules);
            Ok(Some(PortPolicy {
                mode: "custom".into(),
                preset: None,
                rules,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_catalog_materializes_exact_existing_ranges_only_on_request() {
        let preset = preset_policy(COMMON_PROXY_PRESET).unwrap();
        assert_eq!(preset.mode, "preset");
        assert_eq!(preset.preset.as_deref(), Some(COMMON_PROXY_PRESET));
        assert_eq!(
            preset
                .rules
                .iter()
                .map(|r| (r.start, r.end))
                .collect::<Vec<_>>(),
            [
                (1080, 1080),
                (7890, 7892),
                (8080, 8080),
                (8888, 8888),
                (9050, 9050),
                (9090, 9090),
                (9150, 9150),
                (10808, 10808)
            ]
        );
        assert!(
            preset
                .rules
                .iter()
                .all(|r| r.protocol == PortProtocol::Both)
        );
        assert!(preset_policy("unknown").is_none());
        let mut stored = PortPolicy {
            mode: "preset".into(),
            preset: Some(COMMON_PROXY_PRESET.into()),
            rules: vec![rule("udp", 443, 443).unwrap()],
        };
        crate::canonical::normalize_port_policy(&mut stored);
        assert_eq!(stored.rules, [rule("udp", 443, 443).unwrap()]);
    }

    #[test]
    fn normalization_preserves_protocols_and_ranges_while_removing_only_exact_duplicates() {
        let tcp = rule("tcp", 8080, 8080).unwrap();
        let udp = rule("udp", 8080, 8080).unwrap();
        let single = rule("both", 1080, 1080).unwrap();
        let range = rule("both", 1080, 1081).unwrap();
        let mut rules = vec![udp, range, tcp, single, udp];
        normalize_rules(&mut rules);
        assert_eq!(rules, [single, range, tcp, udp]);
    }

    #[test]
    fn agent_modes_validate_only_the_selected_intent_and_keep_legacy_label_semantics() {
        let bad = [PortRuleIntent {
            protocol: "udp".into(),
            start: -1,
            end: 80,
        }];
        assert_eq!(parse_agent_intent(" ALL ", Some("ignored"), &bad), Ok(None));
        assert_eq!(
            parse_agent_intent("preset", Some(COMMON_PROXY_PRESET), &bad).unwrap(),
            preset_policy(COMMON_PROXY_PRESET)
        );
        assert_eq!(
            parse_agent_intent("preset", None, &[]).unwrap_err(),
            "Port preset is required"
        );
        assert_eq!(
            parse_agent_intent("preset", Some("missing"), &[]).unwrap_err(),
            "Unknown port preset: missing"
        );
        assert_eq!(
            parse_agent_intent("custom", None, &[]).unwrap_err(),
            "Custom port policy requires at least one rule"
        );
        assert!(parse_agent_intent("custom", None, &bad).is_err());
        let policy = parse_agent_intent(
            "legacy-label",
            Some("not-used"),
            &[PortRuleIntent {
                protocol: " OTHER ".into(),
                start: 443,
                end: 443,
            }],
        )
        .unwrap()
        .unwrap();
        assert_eq!(policy.mode, "custom");
        assert_eq!(policy.preset, None);
        assert_eq!(policy.rules, [rule("both", 443, 443).unwrap()]);
    }

    #[test]
    fn shared_range_validation_cannot_truncate_signed_or_large_inputs() {
        for (start, end) in [
            (-1, 80),
            (0, 80),
            (65536, 65536),
            (80, -1),
            (80, 0),
            (80, 65536),
            (9000, 8000),
            (i64::MAX, i64::MAX),
        ] {
            assert!(rule("tcp", start, end).is_err(), "{start}..{end}");
        }
        assert_eq!(
            rule("udp", 1, 65535).unwrap(),
            PortRule {
                protocol: PortProtocol::Udp,
                start: 1,
                end: 65535
            }
        );
    }
}
