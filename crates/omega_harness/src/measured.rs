//! A digest that can only exist because the host hashed bytes.
//!
//! The whole packet turns on this file. A maintenance receipt is worth
//! something only if the digest in it is a *measurement* — a number the host
//! computed from the bytes that are actually going to run — rather than a
//! *claim* copied out of a registry document, a release tag, or a request that
//! arrived over a wire.
//!
//! Rust cannot tell those apart at the type level unless the type refuses to be
//! built from a string. So [`MeasuredDigest`] has exactly one primitive
//! constructor, [`MeasuredDigest::measure`], and it takes bytes. There is no
//! `From<String>`, no `new(&str)`, no `FromStr`, and deliberately no
//! `Deserialize`: a value that arrived as text is not a measurement this host
//! made, and giving it the same type would erase the only distinction that
//! matters.
//!
//! [`MeasuredDigest::measure_tree`] is the second constructor and it is not an
//! exception: its inputs are themselves `MeasuredDigest` values, so the only
//! way into the type is still through bytes.
//!
//! `OMEGA-DELTA-0025` pins that shape mechanically, because it is the kind of
//! invariant a convenience constructor added in good faith would silently
//! undo.

use sha2::{Digest as _, Sha256};

/// A SHA-256 digest of bytes this host read.
///
/// Compared against a pin and written into a receipt. It is never parsed from
/// one: see the module documentation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeasuredDigest(String);

impl MeasuredDigest {
    /// Hash bytes the host read.
    ///
    /// The only way to obtain a `MeasuredDigest` from anything that is not
    /// already one.
    #[must_use]
    pub fn measure(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Self(format!("{:x}", hasher.finalize()))
    }

    /// Fold a set of already-measured files into one digest for the whole
    /// installed payload.
    ///
    /// An installed harness is a directory, not a file. Measuring only the
    /// executable named by `cmd` would leave every sidecar in the same
    /// directory unattested, and a swap that replaced one of those would
    /// produce an unchanged digest. So the attested unit is the tree.
    ///
    /// The canonical form sorts by path and separates the path from its digest
    /// with a NUL, which no path component can contain, so two different trees
    /// cannot fold to the same input string.
    #[must_use]
    pub fn measure_tree(files: &mut [(String, MeasuredDigest)]) -> Self {
        files.sort();
        let mut canonical = String::new();
        for (path, digest) in files.iter() {
            canonical.push_str(path);
            canonical.push('\0');
            canonical.push_str(digest.as_str());
            canonical.push('\n');
        }
        Self::measure(canonical.as_bytes())
    }

    /// The lowercase hexadecimal digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether a recorded digest — a pin, or a digest decoded from an older
    /// receipt — describes the same bytes this host just measured.
    ///
    /// Takes the claim as a `&str` on purpose. Comparing a measurement to a
    /// claim is exactly what a pin check is; *becoming* one is what
    /// `MeasuredDigest` refuses.
    #[must_use]
    pub fn matches_recorded(&self, recorded: &str) -> bool {
        self.0.eq_ignore_ascii_case(recorded.trim())
    }
}

impl std::fmt::Display for MeasuredDigest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned against an independently computed value, so a change of hash
    /// function or of encoding is caught rather than merely staying
    /// self-consistent. Produced by `printf 'omega' | shasum -a 256`.
    #[test]
    fn a_digest_is_the_sha256_of_the_bytes() {
        assert_eq!(
            MeasuredDigest::measure(b"omega").as_str(),
            "304b4a90a76a1cbe4c112e074b30e75181f54df43d60f883597457844293b341"
        );
    }

    #[test]
    fn measuring_the_same_bytes_twice_agrees_and_different_bytes_do_not() {
        let first = MeasuredDigest::measure(b"codex-acp v0.9.4");
        let second = MeasuredDigest::measure(b"codex-acp v0.9.4");
        let other = MeasuredDigest::measure(b"codex-acp v0.9.5");
        assert_eq!(first, second);
        assert_ne!(first, other);
        assert_eq!(first.as_str().len(), 64);
        assert!(first.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// The fold has to be sensitive to *where* a byte moved, not only to the
    /// multiset of file contents. Two trees whose contents are swapped between
    /// two paths are different trees.
    #[test]
    fn the_tree_digest_binds_content_to_its_path() {
        let alpha = MeasuredDigest::measure(b"alpha");
        let beta = MeasuredDigest::measure(b"beta");
        let forward = MeasuredDigest::measure_tree(&mut [
            ("bin/agent".to_string(), alpha.clone()),
            ("lib/support".to_string(), beta.clone()),
        ]);
        let swapped = MeasuredDigest::measure_tree(&mut [
            ("bin/agent".to_string(), beta),
            ("lib/support".to_string(), alpha),
        ]);
        assert_ne!(forward, swapped);
    }

    #[test]
    fn the_tree_digest_does_not_depend_on_the_order_the_host_walked_the_directory() {
        let alpha = MeasuredDigest::measure(b"alpha");
        let beta = MeasuredDigest::measure(b"beta");
        let one = MeasuredDigest::measure_tree(&mut [
            ("bin/agent".to_string(), alpha.clone()),
            ("lib/support".to_string(), beta.clone()),
        ]);
        let two = MeasuredDigest::measure_tree(&mut [
            ("lib/support".to_string(), beta),
            ("bin/agent".to_string(), alpha),
        ]);
        assert_eq!(one, two);
    }

    /// An extra file in the installed tree changes the tree digest even when
    /// every previously-present file is byte-identical.
    ///
    /// The added path sorts *after* the existing one on purpose. An earlier
    /// version of this test added `bin/.hidden`, which sorts first, so a fold
    /// that only consumed the first entry still produced a different digest and
    /// the test passed while covering nothing. That mutation was found by
    /// falsifying it.
    #[test]
    fn adding_a_file_to_the_tree_changes_the_tree_digest() {
        let alpha = MeasuredDigest::measure(b"alpha");
        let before = MeasuredDigest::measure_tree(&mut [("bin/agent".to_string(), alpha.clone())]);
        let after = MeasuredDigest::measure_tree(&mut [
            ("bin/agent".to_string(), alpha),
            ("lib/plugin.so".to_string(), MeasuredDigest::measure(b"")),
        ]);
        assert_ne!(before, after);
    }

    /// The whole tree reaches the digest, not a prefix of it.
    ///
    /// This is the assertion the three tests above could not make: every one of
    /// them compares trees that already differ in their first entry, so a fold
    /// that stopped after the first file passed all three. Here only the *last*
    /// file changes, so a truncated fold produces two equal digests and the
    /// test goes red.
    #[test]
    fn changing_only_the_last_file_in_the_tree_changes_the_digest() {
        let first = MeasuredDigest::measure(b"first");
        let one = MeasuredDigest::measure_tree(&mut [
            ("a/first".to_string(), first.clone()),
            ("z/last".to_string(), MeasuredDigest::measure(b"before")),
        ]);
        let two = MeasuredDigest::measure_tree(&mut [
            ("a/first".to_string(), first),
            ("z/last".to_string(), MeasuredDigest::measure(b"after")),
        ]);
        assert_ne!(
            one, two,
            "a file the fold never reached is a file nothing attests"
        );
    }

    #[test]
    fn a_recorded_digest_is_compared_case_and_whitespace_insensitively() {
        let measured = MeasuredDigest::measure(b"omega");
        assert!(measured.matches_recorded(&measured.as_str().to_uppercase()));
        assert!(measured.matches_recorded(&format!("  {}  ", measured.as_str())));
        assert!(!measured.matches_recorded("not-a-digest"));
        assert!(!measured.matches_recorded(""));
    }
}
