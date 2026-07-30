//! Public, Nostr-only identity contract for Omega.
//!
//! The secret-bearing wrapper is intentionally inaccessible outside this crate:
//!
//! ```compile_fail
//! use omega_identity::secret::SecretKeyMaterial;
//! ```

mod account_activation;
mod authentication;
mod contract;
mod custody;
mod mutation_lock;
mod proof;
mod public_store;
mod recovery;
mod recovery_artifact;
mod secret;

pub use account_activation::*;
pub use authentication::*;
pub use contract::*;
pub use custody::*;
pub use proof::*;
pub use public_store::*;
pub use recovery::*;
pub use secret::{ImportedSecret, InvalidImportedSecret};
