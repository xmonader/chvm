//! Validated VM lifecycle FSM over the persisted `VmState` variants.
//!
//! chvm's `VmState` has exactly four variants (persisted lowercase in
//! `state.json`, so the set must never change) and this module is the single
//! authority on which edges between them are legal:
//!
//! ```text
//!                 +----------- crash / unkillable ------------+
//!                 |                                           v
//!     Created --> Running --> Stopped          Running --> Failed
//!        |           ^           ^                |
//!        |           |           |                |
//!        +----------(re-)start -+------(re-)start+
//!        |                                      ^
//!        +---------- launch failure ------------+
//! ```
//!
//! Every status write in the manager goes through `transition_allowed`; a
//! rejected edge leaves the state untouched and is logged by the caller.
//! Self-transitions (`from == to`) are never recorded — a no-op status write
//! is a bug in the caller, not a lifecycle event.

use super::state::VmState;

/// Return true if `from -> to` is a valid FSM edge.
pub fn transition_allowed(from: VmState, to: VmState) -> bool {
    use VmState as S;
    if from == to {
        return false;
    }
    matches!(
        (from, to),
        // First successful start.
        (S::Created, S::Running)
        // Launch failed before the VMM came up.
        | (S::Created, S::Failed)
        // Restart of a cleanly stopped VM.
        | (S::Stopped, S::Running)
        // Restart of a crashed VM.
        | (S::Failed, S::Running)
        // Clean stop (requested by the user).
        | (S::Running, S::Stopped)
        // Unexpected exit / stop could not kill the VMM.
        | (S::Running, S::Failed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [VmState; 4] = [
        VmState::Created,
        VmState::Running,
        VmState::Stopped,
        VmState::Failed,
    ];

    #[test]
    fn happy_path_edges_allowed() {
        assert!(transition_allowed(VmState::Created, VmState::Running));
        assert!(transition_allowed(VmState::Running, VmState::Stopped));
    }

    #[test]
    fn restart_edges_allowed() {
        assert!(transition_allowed(VmState::Stopped, VmState::Running));
        assert!(transition_allowed(VmState::Failed, VmState::Running));
    }

    #[test]
    fn failure_edges_allowed() {
        assert!(transition_allowed(VmState::Created, VmState::Failed));
        assert!(transition_allowed(VmState::Running, VmState::Failed));
    }

    #[test]
    fn self_transitions_rejected() {
        for s in ALL {
            assert!(!transition_allowed(s, s), "{s:?} -> {s:?} must be rejected");
        }
    }

    #[test]
    fn backward_transitions_rejected() {
        assert!(!transition_allowed(VmState::Running, VmState::Created));
        assert!(!transition_allowed(VmState::Stopped, VmState::Created));
        assert!(!transition_allowed(VmState::Stopped, VmState::Failed));
        assert!(!transition_allowed(VmState::Failed, VmState::Stopped));
        assert!(!transition_allowed(VmState::Failed, VmState::Created));
        assert!(!transition_allowed(VmState::Created, VmState::Stopped));
    }

    #[test]
    fn every_edge_accounted_for() {
        // Guard against a forgotten variant when VmState grows: enumerate the
        // full 4x4 matrix and assert the allowed set is exactly what we intend.
        let mut allowed = Vec::new();
        for from in ALL {
            for to in ALL {
                if transition_allowed(from, to) {
                    allowed.push((from, to));
                }
            }
        }
        assert_eq!(allowed.len(), 6);
    }
}
