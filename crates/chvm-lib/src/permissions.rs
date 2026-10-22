//! Privilege model for chvm.
//!
//! chvm manipulates `/dev/kvm`, TAP devices, and iptables, so mutating
//! operations require root (euid 0). Read-only operations (`list`,
//! `inspect`, `logs`, `stats`, `images`, `events`, `console`) are also
//! available to members of the `chvm` group, created by `chvm setup`, so
//! operators can observe VMs without full root.

/// Group granted read-only access to chvm state.
pub const ACCESS_GROUP: &str = "chvm";

pub fn is_root() -> bool {
    nix::unistd::geteuid().is_root()
}

/// Whether the effective user belongs to the `chvm` access group, either via
/// a supplementary group or the primary group.
pub fn in_access_group() -> bool {
    let group = match nix::unistd::Group::from_name(ACCESS_GROUP) {
        Ok(Some(group)) => group,
        _ => return false,
    };

    if nix::unistd::getegid() == group.gid {
        return true;
    }

    nix::unistd::getgroups()
        .map(|groups| groups.contains(&group.gid))
        .unwrap_or(false)
}

/// Gate for read-only operations: root or `chvm` group member.
pub fn check_read_access() -> Result<(), String> {
    if is_root() || in_access_group() {
        return Ok(());
    }
    Err(format!(
        "permission denied: read-only chvm access requires root or membership in the '{ACCESS_GROUP}' group\n\
         Add yourself with: sudo usermod -aG {ACCESS_GROUP} $USER (then log in again)"
    ))
}

/// Gate for mutating operations (run/create/stop/rm/setup/exec/attach/...).
pub fn check_mutation_access() -> Result<(), String> {
    if is_root() {
        return Ok(());
    }
    Err(
        "permission denied: chvm mutating operations require root (they manage /dev/kvm, TAP devices, and iptables)\n\
         Re-run via doas/sudo, e.g.: sudo chvm <command>"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_check_matches_euid() {
        assert_eq!(is_root(), nix::unistd::geteuid().is_root());
    }

    #[test]
    fn group_lookup_does_not_panic_for_missing_group() {
        // The chvm group may not exist on every machine; the check must
        // simply report no access rather than fail.
        let _ = in_access_group();
    }

    #[test]
    fn mutation_error_mentions_privilege_escalation() {
        if is_root() {
            eprintln!("running as root — cannot test the unprivileged error path");
            return;
        }
        let err = check_mutation_access().unwrap_err();
        assert!(err.contains("doas/sudo"));
    }

    #[test]
    fn read_error_mentions_the_access_group() {
        if is_root() || in_access_group() {
            eprintln!("already privileged — cannot test the denied path");
            return;
        }
        let err = check_read_access().unwrap_err();
        assert!(err.contains(ACCESS_GROUP));
    }
}
