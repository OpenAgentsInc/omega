//! What `omega --uninstall` removes. OMEGA-DELTA-0036.
//!
//! Up to `0.2.0-rc14` this path ran upstream's uninstaller unchanged. A flag
//! advertised as "Uninstall Omega from user system" removed the *other*
//! editor's application bundle, its whole application-support tree, its logs,
//! its preferences and its remote-server directory, told the user that editor
//! had been uninstalled, and removed no Omega path whatsoever. Omega survived
//! the uninstall intact.
//!
//! The defect was not a typo in a path. It was that the paths lived in a
//! hand-written table in a shell script, disconnected from the code that
//! creates those directories, so nothing in the tree could notice that the two
//! had never agreed. So there is no table here either: every root comes from
//! the `paths::` function that produced it, and the plan is a value the tests
//! can read.

use std::path::{Path, PathBuf};

/// The roots an Omega installation occupies on this machine.
///
/// One field per place Omega writes. `plan` destructures this exhaustively, so
/// a root added here without being planned does not compile — which is the
/// property the old shell table could not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallRoots {
    /// The application bundle or executable this CLI belongs to.
    pub app: Option<PathBuf>,
    /// `paths::data_dir()` — database, extensions, embeddings.
    pub data_dir: PathBuf,
    /// `paths::config_dir()` — settings and keymap. Prompted for, never
    /// removed silently: it is the only root a user may want to keep.
    pub config_dir: PathBuf,
    /// `paths::logs_dir()` — on macOS this is `~/Library/Logs/<slug>`, not a
    /// subdirectory of the data root. The tripwire harness got this wrong in
    /// the other direction (omega#90).
    pub logs_dir: PathBuf,
    /// `paths::temp_dir()` — the cache root.
    pub temp_dir: PathBuf,
    /// `paths::state_dir()`.
    pub state_dir: PathBuf,
    /// The `omega` symlink an earlier "install CLI" put on `PATH`.
    pub cli_symlink: PathBuf,
    /// Paths keyed by the macOS bundle identifier: preferences, HTTP storage,
    /// saved application state. Empty off macOS.
    pub platform_paths: Vec<PathBuf>,
}

/// The display name and the paths, ready to hand to `script/uninstall.sh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallPlan {
    /// Channel display name, e.g. `Omega RC`.
    pub product: String,
    /// Everything removed without asking.
    pub removals: Vec<PathBuf>,
    /// Removed only if the user says so.
    pub config_dir: PathBuf,
}

impl UninstallRoots {
    /// The roots of the installation this CLI binary belongs to.
    ///
    /// Every field is read from the function that writes it. Nothing here is a
    /// literal path.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn from_installed_paths(app: Option<PathBuf>) -> Self {
        let app_id = release_channel::RELEASE_CHANNEL.app_id();
        let home = paths::home_dir();
        let platform_paths = if cfg!(target_os = "macos") {
            vec![
                home.join("Library/HTTPStorages").join(app_id),
                home.join("Library/Preferences").join(format!("{app_id}.plist")),
                home.join("Library/Saved Application State")
                    .join(format!("{app_id}.savedState")),
            ]
        } else {
            vec![
                home.join(".local/share/applications")
                    .join(format!("{app_id}.desktop")),
            ]
        };
        Self {
            app,
            data_dir: paths::data_dir().clone(),
            config_dir: paths::config_dir().clone(),
            logs_dir: paths::logs_dir().clone(),
            temp_dir: paths::temp_dir().clone(),
            state_dir: paths::state_dir().clone(),
            cli_symlink: home.join(".local/bin").join(paths::BINARY_NAME),
            platform_paths,
        }
    }

    /// Turn the roots into a plan, de-duplicated and in a stable order.
    ///
    /// The config directory is held back deliberately: it is the user's
    /// settings and keymap, and the caller asks before removing it.
    pub fn plan(&self, product: &str) -> UninstallPlan {
        let Self {
            app,
            data_dir,
            config_dir,
            logs_dir,
            temp_dir,
            state_dir,
            cli_symlink,
            platform_paths,
        } = self;

        let mut removals: Vec<PathBuf> = Vec::new();
        let mut push = |path: &Path| {
            let path = path.to_path_buf();
            if path != *config_dir && !removals.contains(&path) {
                removals.push(path);
            }
        };
        if let Some(app) = app {
            push(app);
        }
        push(data_dir);
        push(logs_dir);
        push(temp_dir);
        push(state_dir);
        push(cli_symlink);
        for path in platform_paths {
            push(path);
        }

        UninstallPlan {
            product: product.to_owned(),
            removals,
            config_dir: config_dir.clone(),
        }
    }
}

impl UninstallPlan {
    /// The newline-separated path list `script/uninstall.sh` reads.
    pub fn paths_env(&self) -> String {
        self.removals
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "These tests run the shipped uninstall script as a subprocess; there is no async runtime here."
)]
mod tests {
    use super::*;

    fn roots(home: &Path) -> UninstallRoots {
        UninstallRoots {
            app: Some(home.join("Applications/Omega.app")),
            data_dir: home.join("Library/Application Support/Omega RC"),
            config_dir: home.join(".config/omega-rc"),
            logs_dir: home.join("Library/Logs/omega-rc"),
            temp_dir: home.join("Library/Caches/omega-rc"),
            state_dir: home.join(".local/state/omega-rc"),
            cli_symlink: home.join(".local/bin/omega"),
            platform_paths: vec![
                home.join("Library/HTTPStorages/com.openagents.omega.rc"),
                home.join("Library/Preferences/com.openagents.omega.rc.plist"),
            ],
        }
    }

    #[test]
    fn the_plan_holds_the_config_directory_back_and_keeps_everything_else() {
        let home = Path::new("/tmp/omega-uninstall-fixture");
        let plan = roots(home).plan("Omega RC");
        assert_eq!(plan.product, "Omega RC");
        assert!(
            !plan.removals.contains(&plan.config_dir),
            "settings and keymap are prompted for, never removed silently"
        );
        for expected in [
            home.join("Applications/Omega.app"),
            home.join("Library/Application Support/Omega RC"),
            home.join("Library/Logs/omega-rc"),
            home.join("Library/Caches/omega-rc"),
            home.join(".local/state/omega-rc"),
            home.join(".local/bin/omega"),
        ] {
            assert!(
                plan.removals.contains(&expected),
                "the plan omits {}, which an installed Omega writes",
                expected.display()
            );
        }
        assert_eq!(
            plan.paths_env().lines().count(),
            plan.removals.len(),
            "one path per line, and no path carrying a newline"
        );
    }

    /// OMEGA-DELTA-0036. The shipped script, run for real, against a machine
    /// that has both editors installed.
    ///
    /// This is the assertion `0.2.0-rc14` would have failed on every count: it
    /// removed none of the Omega tree and all of the other one.
    #[test]
    fn the_script_removes_omega_and_leaves_the_other_editor_untouched() {
        let temp = tempfile::tempdir().expect("temp dir");
        let home = temp.path();
        let roots = roots(home);

        // An Omega installation.
        let mut planted = Vec::new();
        for directory in [
            &roots.app.clone().unwrap(),
            &roots.data_dir,
            &roots.config_dir,
            &roots.logs_dir,
            &roots.temp_dir,
            &roots.state_dir,
        ] {
            std::fs::create_dir_all(directory).expect("create omega root");
            let file = directory.join("marker");
            std::fs::write(&file, b"omega").expect("write omega marker");
            planted.push(file);
        }
        std::fs::create_dir_all(roots.cli_symlink.parent().unwrap()).unwrap();
        std::fs::write(&roots.cli_symlink, b"omega").unwrap();
        for path in &roots.platform_paths {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"omega").unwrap();
        }

        // The other editor, installed alongside it. Read back byte for byte
        // afterwards: "we did not mean to" is not an observation.
        let foreign: Vec<PathBuf> = [
            "Applications/OtherEditor.app/Contents/Info.plist",
            "Library/Application Support/OtherEditor/db/state",
            "Library/Logs/OtherEditor/log.txt",
            ".config/othereditor/settings.json",
            ".othereditor_server/bin",
        ]
        .iter()
        .map(|relative| home.join(relative))
        .collect();
        for path in &foreign {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"not ours to remove").unwrap();
        }

        let plan = roots.plan("Omega RC");
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../script/uninstall.sh")
            .canonicalize()
            .expect("the shipped uninstall script");
        let output = std::process::Command::new("sh")
            .arg(&script)
            .env("HOME", home)
            .env("OMEGA_UNINSTALL_PRODUCT", &plan.product)
            .env("OMEGA_UNINSTALL_PATHS", plan.paths_env())
            .env("OMEGA_UNINSTALL_CONFIG_DIR", &plan.config_dir)
            .env("OMEGA_UNINSTALL_ASSUME_YES", "1")
            .output()
            .expect("run the uninstall script");
        assert!(
            output.status.success(),
            "uninstall failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Read against the ROOTS, not against the plan. Asserting that
        // everything the plan listed is gone is circular: a plan that forgot a
        // root would pass it. Destructuring is exhaustive, so a root added to
        // the struct and left out of the plan fails to compile here.
        let UninstallRoots {
            app,
            data_dir,
            config_dir,
            logs_dir,
            temp_dir,
            state_dir,
            cli_symlink,
            platform_paths,
        } = &roots;
        let occupied: Vec<&PathBuf> = app
            .iter()
            .chain([data_dir, config_dir, logs_dir, temp_dir, state_dir, cli_symlink])
            .chain(platform_paths.iter())
            .collect();
        for path in occupied {
            assert!(
                !path.exists(),
                "{} survived the uninstall, so Omega was not removed",
                path.display()
            );
        }
        for path in &foreign {
            assert_eq!(
                std::fs::read(path).ok().as_deref(),
                Some(b"not ours to remove".as_slice()),
                "{} was touched; the uninstall reached into another product",
                path.display()
            );
        }
    }

    /// Every test in this module runs the real uninstaller, so every one of
    /// them gets a `HOME` of its own.
    ///
    /// This is not hygiene. On 2026-07-25 a lane falsifying this delta restored
    /// the `0.2.0-rc14` script over this file and ran the tests; the refusal
    /// test below did not override `HOME`, the restored script ignored the
    /// plan entirely, and it deleted the real machine's other editor and its
    /// whole application-support tree. A test that runs an uninstaller must
    /// never be able to see the real home directory, whatever the script under
    /// it happens to be that minute.
    #[test]
    fn the_script_refuses_an_empty_or_relative_plan() {
        let temp = tempfile::tempdir().expect("temp dir");
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../script/uninstall.sh");
        for (paths, why) in [
            ("", "an empty plan"),
            ("Library/Application Support/Omega RC", "a relative path"),
            ("/", "the root of the filesystem"),
            ("/Applications", "a whole system directory"),
        ] {
            let output = std::process::Command::new("sh")
                .arg(&script)
                .env("HOME", temp.path())
                .env("OMEGA_UNINSTALL_PRODUCT", "Omega RC")
                .env("OMEGA_UNINSTALL_PATHS", paths)
                .env("OMEGA_UNINSTALL_ASSUME_YES", "1")
                .output()
                .expect("run the uninstall script");
            assert!(
                !output.status.success(),
                "the script accepted {why}; refusing is the safe direction"
            );
        }
    }

    #[test]
    fn the_uninstall_script_names_no_other_product() {
        let script = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../script/uninstall.sh"),
        )
        .expect("the shipped uninstall script");
        let lowered = script.to_lowercase();
        assert!(
            !lowered.contains("zed"),
            "script/uninstall.sh names a competitor's product. It ships inside \
             the signed `cli` binary and removes whatever it names."
        );
        assert!(
            script.contains("OMEGA_UNINSTALL_PATHS"),
            "the script no longer reads its plan from the caller, so it has a \
             path table again, which is what shipped omega#88"
        );
    }
}
