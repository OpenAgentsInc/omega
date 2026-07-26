//! Exo stays on the machine it runs on. `OMEGA-DELTA-0042`, omega#87.
//!
//! `exo serve` is a single unary HTTP endpoint with **no authentication**. Its
//! own documentation says so: a client may send a bearer token and the server
//! never checks it. It exposes the full 53-variant request protocol, which
//! includes reading secrets. Loopback is the entire boundary, and Exo knows it.
//!
//! Omega therefore treats "where Exo listens" and "where Omega talks to Exo" as
//! typed values that cannot be built out of a non-loopback address, rather than
//! as strings a caller is trusted to have checked. The refuse list in the
//! teardown is stated as a law here: *do not expose Exo's unauthenticated HTTP
//! endpoint or agent-cli socket beyond loopback through any Omega surface.*
//!
//! # Why a parser rather than a check
//!
//! A check is something a call site can forget. [`LoopbackEndpoint`] has one
//! constructor, it takes a string, and it fails. Every value of the type is a
//! loopback address because there is no other way to make one — the same
//! discipline `MeasuredDigest` uses for bytes.
//!
//! # What "loopback" admits
//!
//! `127.0.0.0/8`, `::1`, and the literal name `localhost`. Nothing else. In
//! particular `0.0.0.0` and `::` are refused, and those are the two that matter:
//! they are the *plausible* mistakes. They read as "local" and they mean "every
//! interface on this machine", which on a laptop that joins a Tailnet or a café
//! network publishes an unauthenticated agent with a shell to that network.

/// An address Omega will let Exo be reached at.
///
/// Loopback by construction. See the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopbackEndpoint {
    host: String,
    port: Option<u16>,
}

/// Why an address was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffLoopback {
    /// The address names every interface — `0.0.0.0` or `::`.
    EveryInterface,
    /// The address names a specific host that is not this machine.
    RemoteHost,
    /// The string is not an address this build can read. Unreadable is refused
    /// rather than assumed local, which is the fail-closed direction.
    Unreadable,
}

impl std::fmt::Display for OffLoopback {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EveryInterface => {
                "that address publishes Exo on every interface, and Exo has no authentication"
            }
            Self::RemoteHost => "Omega only reaches Exo on this machine",
            Self::Unreadable => "that is not an address Omega can read as loopback",
        })
    }
}

impl std::error::Error for OffLoopback {}

/// Where `exo serve` listens when nobody says otherwise.
///
/// Exo's own default, restated here so the lane's default is a value in Omega's
/// source rather than a behaviour inherited from an unstable upstream.
pub const EXO_SERVE_DEFAULT_BIND: &str = "127.0.0.1:4766";

impl LoopbackEndpoint {
    /// Read an address, admitting only loopback.
    ///
    /// # Errors
    ///
    /// [`OffLoopback`] for anything that is not this machine.
    pub fn parse(address: &str) -> Result<Self, OffLoopback> {
        let address = address.trim();
        if address.is_empty() {
            return Err(OffLoopback::Unreadable);
        }

        // A URL is reduced to its authority before anything else, so
        // `http://127.0.0.1:4766/request` and `127.0.0.1:4766` are the same
        // decision. A scheme this build does not know is unreadable, not
        // assumed harmless.
        let authority = match address.split_once("://") {
            Some((scheme, rest)) => {
                if !matches!(scheme, "http" | "https") {
                    return Err(OffLoopback::Unreadable);
                }
                rest.split(['/', '?', '#']).next().unwrap_or(rest)
            }
            None => address,
        };
        if authority.contains('@') {
            // Userinfo can hide the real host behind an `@`. Refuse rather than
            // pick a side of it.
            return Err(OffLoopback::Unreadable);
        }

        let (host, port) = split_host_port(authority)?;
        match classify_host(&host) {
            HostKind::Loopback => Ok(Self { host, port }),
            HostKind::EveryInterface => Err(OffLoopback::EveryInterface),
            HostKind::Remote => Err(OffLoopback::RemoteHost),
            HostKind::Unreadable => Err(OffLoopback::Unreadable),
        }
    }

    /// The host, as read.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, when the address carried one.
    #[must_use]
    pub const fn port(&self) -> Option<u16> {
        self.port
    }
}

impl std::fmt::Display for LoopbackEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.port {
            Some(port) if self.host.contains(':') => write!(formatter, "[{}]:{port}", self.host),
            Some(port) => write!(formatter, "{}:{port}", self.host),
            None => formatter.write_str(&self.host),
        }
    }
}

enum HostKind {
    Loopback,
    EveryInterface,
    Remote,
    Unreadable,
}

fn split_host_port(authority: &str) -> Result<(String, Option<u16>), OffLoopback> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest.split_once(']').ok_or(OffLoopback::Unreadable)?;
        let port = match tail.strip_prefix(':') {
            Some(port) => Some(port.parse().map_err(|_| OffLoopback::Unreadable)?),
            None if tail.is_empty() => None,
            None => return Err(OffLoopback::Unreadable),
        };
        return Ok((host.to_ascii_lowercase(), port));
    }
    // A bare IPv6 literal carries colons and no port.
    if authority.matches(':').count() > 1 {
        return Ok((authority.to_ascii_lowercase(), None));
    }
    match authority.split_once(':') {
        Some((host, port)) => Ok((
            host.to_ascii_lowercase(),
            Some(port.parse().map_err(|_| OffLoopback::Unreadable)?),
        )),
        None => Ok((authority.to_ascii_lowercase(), None)),
    }
}

fn classify_host(host: &str) -> HostKind {
    if host == "localhost" {
        return HostKind::Loopback;
    }
    if host == "0.0.0.0" || host == "::" || host == "[::]" {
        return HostKind::EveryInterface;
    }
    if host == "::1" {
        return HostKind::Loopback;
    }
    let octets: Vec<&str> = host.split('.').collect();
    if octets.len() == 4 && octets.iter().all(|part| part.parse::<u8>().is_ok()) {
        return if octets[0] == "127" {
            HostKind::Loopback
        } else {
            HostKind::Remote
        };
    }
    if host.contains(':') {
        // Any other IPv6 literal. `::ffff:0.0.0.0` and friends are deliberately
        // not decoded into their embedded IPv4 form: a lane that tried would be
        // making a routing judgement, and refusing is the fail-closed answer.
        return HostKind::Remote;
    }
    if host.is_empty() {
        return HostKind::Unreadable;
    }
    HostKind::Remote
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exos_own_default_bind_is_loopback_and_is_admitted() {
        let endpoint = LoopbackEndpoint::parse(EXO_SERVE_DEFAULT_BIND).expect("loopback");
        assert_eq!(endpoint.host(), "127.0.0.1");
        assert_eq!(endpoint.port(), Some(4766));
        assert_eq!(endpoint.to_string(), EXO_SERVE_DEFAULT_BIND);
    }

    #[test]
    fn every_loopback_spelling_is_admitted() {
        for spelling in [
            "127.0.0.1",
            "127.0.0.1:4766",
            "127.1.2.3:4766",
            "localhost:4766",
            "LOCALHOST",
            "::1",
            "[::1]:4766",
            "http://127.0.0.1:4766/request",
            "http://localhost:4766",
        ] {
            assert!(
                LoopbackEndpoint::parse(spelling).is_ok(),
                "{spelling} is loopback"
            );
        }
    }

    /// The two plausible mistakes. Both read as "local" and neither is.
    #[test]
    fn binding_every_interface_is_refused() {
        for spelling in ["0.0.0.0:4766", "0.0.0.0", "::", "[::]:4766"] {
            assert_eq!(
                LoopbackEndpoint::parse(spelling),
                Err(OffLoopback::EveryInterface),
                "{spelling}"
            );
        }
    }

    #[test]
    fn a_remote_host_is_refused() {
        for spelling in [
            "10.0.0.4:4766",
            "100.64.7.9:4766",
            "192.168.1.20",
            "exo.example.com:4766",
            "http://exo.openagents.com/request",
            "[fd7a:115c::1]:4766",
        ] {
            assert_eq!(
                LoopbackEndpoint::parse(spelling),
                Err(OffLoopback::RemoteHost),
                "{spelling}"
            );
        }
    }

    /// A host hidden behind userinfo, an unknown scheme, and an empty string
    /// are refused rather than guessed at. Unreadable fails closed.
    #[test]
    fn an_address_this_build_cannot_read_is_refused_rather_than_assumed_local() {
        for spelling in [
            "http://127.0.0.1@evil.example.com/request",
            "ssh://127.0.0.1:4766",
            "",
            "   ",
            "127.0.0.1:notaport",
        ] {
            assert_eq!(
                LoopbackEndpoint::parse(spelling),
                Err(OffLoopback::Unreadable),
                "{spelling:?}"
            );
        }
    }
}
