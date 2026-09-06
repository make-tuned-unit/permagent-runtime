//! Observable context-packet accounting for coding turns.
//!
//! The packet is a projection, not a second memory system.  Spectral remains
//! the only source of durable memory; this module only records what a caller
//! placed in one model request.  It is intentionally pure so tests never need
//! a provider, database, embedder, or network connection.
//!
//! Every packet has the same five slots: fixed prompt, tool schema, project
//! memory, retrieved Spectral memory, and tool output.  An absent slot is
//! represented as `Missing` rather than as zero tokens, avoiding the common
//! mistake of reading unavailable context as an empty context.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The context source represented by one packet slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    FixedPrompt,
    ToolSchema,
    ProjectMemory,
    SpectralMemory,
    ToolOutput,
}

impl ContextSource {
    pub const ALL: [Self; 5] = [
        Self::FixedPrompt,
        Self::ToolSchema,
        Self::ProjectMemory,
        Self::SpectralMemory,
        Self::ToolOutput,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FixedPrompt => "fixed_prompt",
            Self::ToolSchema => "tool_schema",
            Self::ProjectMemory => "project_memory",
            Self::SpectralMemory => "spectral_memory",
            Self::ToolOutput => "tool_output",
        }
    }
}

/// Why a packet slot has no text.  This is explicit so dashboards can
/// distinguish “not configured” from a measured zero-length payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Present,
    Missing { reason: String },
}

/// A fact supplied by the fixed harness contract.  Protected facts are
/// identified by stable names and are never merged with dynamic memory text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedFact {
    pub name: String,
    pub value: String,
}

/// One retrieved Spectral memory before packet assembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpectralMemory<'a> {
    /// Stable Spectral memory key/id. Duplicate keys are collapsed first-win.
    pub key: &'a str,
    pub text: &'a str,
    /// Retrieval provenance refs (for example `retrieval:<uuid>` or
    /// `recognition:<uuid>`). Duplicate refs are retained once, in order.
    pub provenance: &'a [&'a str],
}

/// Owned form useful at process boundaries (for example the coding CLI's
/// authenticated Brain bridge). It preserves the same deterministic assembly
/// rules without borrowing a response buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpectralMemoryRecord {
    pub key: String,
    pub text: String,
    pub provenance: Vec<String>,
}

/// Typed attribution for text installed in a system-prompt extra.
///
/// The prompt is intentionally not parsed at the request seam. Producers that
/// know where a block came from register that fact alongside the block, and
/// the packet projects those registrations into its source slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextContribution {
    pub source: ContextSource,
    /// Stable key for a retrieved Spectral memory. Project-level contributions
    /// may leave this unset.
    pub key: Option<String>,
    pub text: String,
    pub provenance: Vec<String>,
}

impl ContextContribution {
    pub fn project_memory(
        text: impl Into<String>,
        provenance: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            source: ContextSource::ProjectMemory,
            key: None,
            text: text.into(),
            provenance: provenance.into_iter().collect(),
        }
    }

    pub fn spectral_memory(
        key: impl Into<String>,
        text: impl Into<String>,
        provenance: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            source: ContextSource::SpectralMemory,
            key: Some(key.into()),
            text: text.into(),
            provenance: provenance.into_iter().collect(),
        }
    }
}

/// One packet slot's measured projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPart {
    pub source: ContextSource,
    pub availability: Availability,
    /// Character count of the exact text represented by this slot.
    pub characters: Option<usize>,
    /// Deterministic estimate: ceil(characters / 4). This is an estimate, not
    /// a provider tokenizer reading.
    pub estimated_tokens: Option<usize>,
    /// Stable provenance refs included in this slot.
    pub provenance: Vec<String>,
    /// Fixed contract material is protected from dynamic context.
    pub protected: bool,
}

/// Machine-readable context accounting for one coding request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPacket {
    pub parts: Vec<ContextPart>,
    pub protected_facts: Vec<ProtectedFact>,
    /// Union of provenance refs from the Spectral retrieval slot.
    pub retrieval_provenance: Vec<String>,
    /// Sum of known slot estimates. `None` if every slot is missing.
    pub estimated_total_tokens: Option<usize>,
}

fn measured_text(source: ContextSource, text: Option<&str>, protected: bool) -> ContextPart {
    let text = text.filter(|value| !value.is_empty());
    let (availability, characters, estimated_tokens) = match text {
        Some(value) => {
            let characters = value.chars().count();
            let estimated_tokens = characters.saturating_add(3) / 4;
            (
                Availability::Present,
                Some(characters),
                Some(estimated_tokens),
            )
        }
        None => (
            Availability::Missing {
                reason: "not supplied".to_string(),
            },
            None,
            None,
        ),
    };
    ContextPart {
        source,
        availability,
        characters,
        estimated_tokens,
        provenance: Vec::new(),
        protected,
    }
}

/// Assemble the five fixed packet slots without performing I/O.
///
/// Spectral memories are deduplicated by stable key, first result wins. Their
/// provenance is deduplicated in first-seen order and exposed both on the
/// memory slot and as the packet-level retrieval union. No non-Spectral memory
/// source can be passed through this API.
pub fn assemble(
    fixed_prompt: Option<&str>,
    tool_schema: Option<&str>,
    project_memory: Option<&str>,
    spectral_memories: &[SpectralMemory<'_>],
    tool_output: Option<&str>,
    protected_facts: &[ProtectedFact],
) -> ContextPacket {
    let mut spectral_text = String::new();
    let mut seen_keys = HashSet::new();
    let mut provenance = Vec::new();
    let mut seen_provenance = HashSet::new();
    for memory in spectral_memories {
        if memory.key.is_empty() || !seen_keys.insert(memory.key) {
            continue;
        }
        if !memory.text.is_empty() {
            if !spectral_text.is_empty() {
                spectral_text.push('\n');
            }
            spectral_text.push_str(memory.text);
        }
        for reference in memory.provenance {
            if !reference.is_empty() && seen_provenance.insert(*reference) {
                provenance.push((*reference).to_string());
            }
        }
    }

    let mut spectral = measured_text(
        ContextSource::SpectralMemory,
        (!spectral_text.is_empty()).then_some(spectral_text.as_str()),
        false,
    );
    spectral.provenance = provenance.clone();
    if spectral_memories.iter().any(|m| !m.key.is_empty()) && spectral.provenance.is_empty() {
        spectral.availability = Availability::Present;
        spectral.characters = Some(spectral.characters.unwrap_or(0));
        spectral.estimated_tokens = Some(spectral.characters.unwrap_or(0).saturating_add(3) / 4);
    }

    let parts = vec![
        measured_text(ContextSource::FixedPrompt, fixed_prompt, true),
        measured_text(ContextSource::ToolSchema, tool_schema, true),
        measured_text(ContextSource::ProjectMemory, project_memory, false),
        spectral,
        measured_text(ContextSource::ToolOutput, tool_output, false),
    ];
    let estimated_total_tokens = parts
        .iter()
        .filter_map(|part| part.estimated_tokens)
        .reduce(usize::saturating_add);
    ContextPacket {
        parts,
        protected_facts: dedupe_facts(protected_facts),
        retrieval_provenance: provenance,
        estimated_total_tokens,
    }
}

/// Assemble from owned Spectral bridge records.
pub fn assemble_owned(
    fixed_prompt: Option<&str>,
    tool_schema: Option<&str>,
    project_memory: Option<&str>,
    spectral_memories: &[SpectralMemoryRecord],
    tool_output: Option<&str>,
    protected_facts: &[ProtectedFact],
) -> ContextPacket {
    let provenance: Vec<Vec<&str>> = spectral_memories
        .iter()
        .map(|memory| memory.provenance.iter().map(String::as_str).collect())
        .collect();
    let borrowed: Vec<SpectralMemory<'_>> = spectral_memories
        .iter()
        .zip(provenance.iter())
        .map(|(memory, provenance)| SpectralMemory {
            key: &memory.key,
            text: &memory.text,
            provenance,
        })
        .collect();
    assemble(
        fixed_prompt,
        tool_schema,
        project_memory,
        &borrowed,
        tool_output,
        protected_facts,
    )
}

/// Assemble a packet while preserving typed attribution registered by prompt
/// producers. This is the request-boundary path; unlike prompt-text parsing it
/// cannot mistake ordinary prose for project memory or Spectral recall.
pub fn assemble_with_contributions(
    fixed_prompt: Option<&str>,
    tool_schema: Option<&str>,
    contributions: &[ContextContribution],
    tool_output: Option<&str>,
    protected_facts: &[ProtectedFact],
) -> ContextPacket {
    let project_memory = contributions
        .iter()
        .filter(|item| item.source == ContextSource::ProjectMemory)
        .map(|item| item.text.as_str())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let spectral_memories = contributions
        .iter()
        .filter(|item| item.source == ContextSource::SpectralMemory)
        .filter_map(|item| {
            Some(SpectralMemoryRecord {
                key: item.key.clone()?,
                text: item.text.clone(),
                provenance: item.provenance.clone(),
            })
        })
        .collect::<Vec<_>>();
    let mut packet = assemble_owned(
        fixed_prompt,
        tool_schema,
        (!project_memory.is_empty()).then_some(project_memory.as_str()),
        &spectral_memories,
        tool_output,
        protected_facts,
    );
    let mut project_provenance = Vec::new();
    let mut seen_provenance = HashSet::new();
    for item in contributions
        .iter()
        .filter(|item| item.source == ContextSource::ProjectMemory)
    {
        for reference in &item.provenance {
            if !reference.is_empty() && seen_provenance.insert(reference) {
                project_provenance.push(reference.clone());
            }
        }
    }
    if let Some(part) = packet
        .parts
        .iter_mut()
        .find(|part| part.source == ContextSource::ProjectMemory)
    {
        part.provenance = project_provenance;
    }
    packet
}

fn dedupe_facts(facts: &[ProtectedFact]) -> Vec<ProtectedFact> {
    let mut seen = HashSet::new();
    facts
        .iter()
        .filter(|fact| !fact.name.is_empty() && seen.insert(fact.name.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(packet: &ContextPacket, source: ContextSource) -> &ContextPart {
        packet
            .parts
            .iter()
            .find(|part| part.source == source)
            .expect("fixed packet slot")
    }

    #[test]
    fn packet_always_exposes_fixed_slots_and_missing_is_not_zero() {
        let packet = assemble(None, Some("tool schema"), None, &[], None, &[]);
        assert_eq!(packet.parts.len(), ContextSource::ALL.len());
        assert_eq!(
            part(&packet, ContextSource::FixedPrompt).availability,
            Availability::Missing {
                reason: "not supplied".to_string()
            }
        );
        assert_eq!(
            part(&packet, ContextSource::FixedPrompt).estimated_tokens,
            None
        );
        assert_eq!(
            part(&packet, ContextSource::ToolSchema).estimated_tokens,
            Some(3)
        );
        assert_eq!(packet.estimated_total_tokens, Some(3));
    }

    #[test]
    fn fixed_contract_and_protected_facts_are_distinct_from_dynamic_context() {
        let facts = vec![
            ProtectedFact {
                name: "workspace_root".into(),
                value: "/repo".into(),
            },
            // Duplicate names are first-win to keep the fixed contract stable.
            ProtectedFact {
                name: "workspace_root".into(),
                value: "/attacker-controlled".into(),
            },
        ];
        let packet = assemble(
            Some("You are the coding harness."),
            None,
            Some("Project memory"),
            &[],
            None,
            &facts,
        );
        assert!(part(&packet, ContextSource::FixedPrompt).protected);
        assert!(part(&packet, ContextSource::ToolSchema).protected);
        assert!(!part(&packet, ContextSource::ProjectMemory).protected);
        assert_eq!(packet.protected_facts, vec![facts[0].clone()]);
    }

    #[test]
    fn spectral_memories_and_provenance_are_stably_deduplicated() {
        let first_refs = ["retrieval:r1", "retrieval:r2", "retrieval:r1"];
        let second_refs = ["retrieval:r2", "retrieval:r3"];
        let memories = [
            SpectralMemory {
                key: "memory-1",
                text: "first memory",
                provenance: &first_refs,
            },
            SpectralMemory {
                key: "memory-1",
                text: "duplicate must not enter packet",
                provenance: &second_refs,
            },
            SpectralMemory {
                key: "memory-2",
                text: "second memory",
                provenance: &second_refs,
            },
        ];
        let packet = assemble(None, None, None, &memories, None, &[]);
        let spectral = part(&packet, ContextSource::SpectralMemory);
        assert_eq!(
            spectral.characters,
            Some("first memory\nsecond memory".chars().count())
        );
        assert_eq!(
            packet.retrieval_provenance,
            vec!["retrieval:r1", "retrieval:r2", "retrieval:r3"]
        );
        assert_eq!(spectral.provenance, packet.retrieval_provenance);
        assert!(!spectral
            .provenance
            .iter()
            .any(|reference| reference.contains("duplicate")));
    }

    #[test]
    fn unicode_token_estimate_is_character_based_and_deterministic() {
        let packet = assemble(Some("🧠🧠🧠🧠🧠"), None, None, &[], None, &[]);
        assert_eq!(
            part(&packet, ContextSource::FixedPrompt).characters,
            Some(5)
        );
        assert_eq!(
            part(&packet, ContextSource::FixedPrompt).estimated_tokens,
            Some(2)
        );
    }

    #[test]
    fn owned_bridge_records_follow_the_same_deduplication_contract() {
        let packet = assemble_owned(
            None,
            None,
            None,
            &[
                SpectralMemoryRecord {
                    key: "m1".into(),
                    text: "one".into(),
                    provenance: vec!["retrieval:r1".into()],
                },
                SpectralMemoryRecord {
                    key: "m1".into(),
                    text: "duplicate".into(),
                    provenance: vec!["retrieval:r2".into()],
                },
            ],
            None,
            &[],
        );
        assert_eq!(
            part(&packet, ContextSource::SpectralMemory).characters,
            Some(3)
        );
        assert_eq!(packet.retrieval_provenance, vec!["retrieval:r1"]);
    }

    #[test]
    fn request_projection_accounts_each_exactly_available_slot() {
        let packet = assemble(
            Some("fixed"),
            Some("tools"),
            Some("project"),
            &[],
            Some("tool result"),
            &[],
        );
        assert_eq!(
            ContextSource::ALL
                .iter()
                .map(|source| part(&packet, *source).estimated_tokens)
                .collect::<Vec<_>>(),
            vec![Some(2), Some(2), Some(2), None, Some(3)]
        );
        assert_eq!(packet.estimated_total_tokens, Some(9));
        assert_eq!(
            part(&packet, ContextSource::SpectralMemory).availability,
            Availability::Missing {
                reason: "not supplied".to_string()
            }
        );
    }

    #[test]
    fn typed_contributions_fill_source_slots_without_prompt_parsing() {
        let packet = assemble_with_contributions(
            Some("fixed instructions containing memory-shaped words"),
            Some("tools"),
            &[
                ContextContribution::project_memory(
                    "ranked project map",
                    ["project:repo_map".to_string()],
                ),
                ContextContribution::spectral_memory(
                    "mem-1",
                    "recalled fact",
                    ["retrieval:r1".to_string()],
                ),
            ],
            None,
            &[],
        );
        assert_eq!(
            part(&packet, ContextSource::ProjectMemory).characters,
            Some("ranked project map".chars().count())
        );
        assert_eq!(
            part(&packet, ContextSource::ProjectMemory).provenance,
            vec!["project:repo_map"]
        );
        assert_eq!(
            part(&packet, ContextSource::SpectralMemory).characters,
            Some("recalled fact".chars().count())
        );
        assert_eq!(packet.retrieval_provenance, vec!["retrieval:r1"]);
    }
}
