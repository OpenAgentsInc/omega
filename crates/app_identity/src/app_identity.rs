use std::{env, str::FromStr, sync::LazyLock};

pub const BINARY_NAME: &str = "omega";

pub static CHANNEL: LazyLock<AppChannel> = LazyLock::new(|| {
    let channel_name = if cfg!(debug_assertions) {
        env::var("ZED_RELEASE_CHANNEL")
            .unwrap_or_else(|_| include_str!("../../zed/RELEASE_CHANNEL").trim().to_string())
    } else {
        include_str!("../../zed/RELEASE_CHANNEL").trim().to_string()
    };

    channel_name
        .parse()
        .unwrap_or_else(|_| panic!("invalid release channel {channel_name}"))
});

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AppChannel {
    Dev,
    Nightly,
    Rc,
    Stable,
}

impl AppChannel {
    pub const ALL: [Self; 4] = [Self::Dev, Self::Nightly, Self::Rc, Self::Stable];

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Dev => "Omega Dev",
            Self::Nightly => "Omega Nightly",
            Self::Rc => "Omega RC",
            Self::Stable => "Omega",
        }
    }

    pub const fn storage_slug(self) -> &'static str {
        match self {
            Self::Dev => "omega-dev",
            Self::Nightly => "omega-nightly",
            Self::Rc => "omega-rc",
            Self::Stable => "omega",
        }
    }

    pub const fn app_id(self) -> &'static str {
        match self {
            Self::Dev => "com.openagents.omega.dev",
            Self::Nightly => "com.openagents.omega.nightly",
            Self::Rc => "com.openagents.omega.rc",
            Self::Stable => "com.openagents.omega",
        }
    }

    pub const fn credential_namespace(self) -> &'static str {
        match self {
            Self::Dev => "com.openagents.omega.credentials.dev",
            Self::Nightly => "com.openagents.omega.credentials.nightly",
            Self::Rc => "com.openagents.omega.credentials.rc",
            Self::Stable => "com.openagents.omega.credentials",
        }
    }

    pub const fn protocol_scheme(self) -> &'static str {
        self.storage_slug()
    }
}

impl FromStr for AppChannel {
    type Err = InvalidAppChannel;

    fn from_str(channel: &str) -> Result<Self, Self::Err> {
        match channel {
            "dev" => Ok(Self::Dev),
            "nightly" => Ok(Self::Nightly),
            "preview" | "rc" => Ok(Self::Rc),
            "stable" => Ok(Self::Stable),
            _ => Err(InvalidAppChannel),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct InvalidAppChannel;

#[cfg(test)]
mod icon_family;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn channel_identity_values_are_unique_and_not_zed() {
        let display_names = AppChannel::ALL.map(AppChannel::display_name);
        let storage_slugs = AppChannel::ALL.map(AppChannel::storage_slug);
        let app_ids = AppChannel::ALL.map(AppChannel::app_id);
        let credential_namespaces = AppChannel::ALL.map(AppChannel::credential_namespace);
        let protocol_schemes = AppChannel::ALL.map(AppChannel::protocol_scheme);

        for values in [
            display_names.as_slice(),
            storage_slugs.as_slice(),
            app_ids.as_slice(),
            credential_namespaces.as_slice(),
            protocol_schemes.as_slice(),
        ] {
            assert_eq!(values.iter().copied().collect::<HashSet<_>>().len(), 4);
            assert!(
                values
                    .iter()
                    .all(|value| !value.to_lowercase().contains("zed"))
            );
        }
    }

    #[test]
    fn preview_maps_to_omega_rc() {
        let channel = "preview".parse::<AppChannel>();
        assert_eq!(channel, Ok(AppChannel::Rc));
        assert_eq!(AppChannel::Rc.display_name(), "Omega RC");
        assert_eq!(AppChannel::Rc.storage_slug(), "omega-rc");
        assert_eq!(AppChannel::Rc.app_id(), "com.openagents.omega.rc");
    }
}
