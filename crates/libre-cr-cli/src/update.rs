//! Update checking — stub for Phase 7. The spec calls for a signed JSON
//! manifest at `https://api.libre-cr.dev/latest`; we'll wire that in once the
//! release pipeline (Phase 7.5) is providing it.

/// Returns the human-readable message printed by `libre-cr update`.
pub fn stub_message() -> String {
    format!(
        "libre-cr {} — auto-update is not implemented yet.\n\
         The release manifest (https://api.libre-cr.dev/latest) and signature\n\
         verification land in Phase 7.5; for now please update via the same\n\
         channel you installed from (brew/scoop/release tarball).",
        env!("CARGO_PKG_VERSION")
    )
}
