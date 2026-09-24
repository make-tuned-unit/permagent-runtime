//! Inference-free Model Review. No transport, model loader, or switch capability.
pub mod findings;
pub mod inventory;
pub mod ledger;
pub mod report;

pub const LOCAL_ONLY: &str = "Discovery was local-only; no upstream metadata or inference requests were made. Live discovery is policy-blocked.";

#[cfg(test)]
mod tests;
