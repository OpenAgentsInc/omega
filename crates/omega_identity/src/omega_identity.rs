//! Public, Nostr-only identity contract for Omega.
//!
//! The secret-bearing wrapper is intentionally inaccessible outside this crate:
//!
//! ```compile_fail
//! use omega_identity::secret::SecretKeyMaterial;
//! ```

mod contract;
mod custody;
mod mutation_lock;
mod public_store;
mod proof;
mod recovery;
mod recovery_artifact;
mod secret;

pub use contract::*;
pub use custody::*;
pub use public_store::*;
pub use proof::*;
pub use recovery::*;
pub use secret::{ImportedSecret, InvalidImportedSecret};
