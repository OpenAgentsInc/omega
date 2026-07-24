use std::sync::{Mutex, MutexGuard};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::KeyringLocator;

static PROCESS_MUTATION_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct IdentityMutationGuard {
    _process_guard: MutexGuard<'static, ()>,
    _platform_guard: PlatformMutationGuard,
}

impl IdentityMutationGuard {
    pub(crate) fn acquire(locator: &KeyringLocator) -> Result<Self, MutationLockError> {
        let process_guard = PROCESS_MUTATION_LOCK
            .lock()
            .map_err(|_| MutationLockError::ProcessLockPoisoned)?;
        let platform_guard = PlatformMutationGuard::acquire(locator.service())?;
        Ok(Self {
            _process_guard: process_guard,
            _platform_guard: platform_guard,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Error)]
pub(crate) enum MutationLockError {
    #[error("the process identity mutation lock is unavailable")]
    ProcessLockPoisoned,
    #[error("the operating-system identity mutation lock is unavailable")]
    PlatformLockUnavailable,
}

fn service_hash(service: &str) -> String {
    hex::encode(Sha256::digest(service.as_bytes()))
}

#[cfg(unix)]
struct PlatformMutationGuard {
    _file: std::fs::File,
}

#[cfg(unix)]
impl PlatformMutationGuard {
    fn acquire(service: &str) -> Result<Self, MutationLockError> {
        use std::{
            fs::{self, OpenOptions, Permissions},
            os::unix::{
                fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
                io::AsRawFd,
            },
            path::PathBuf,
        };

        let user_id = unsafe { libc::getuid() };
        let lock_directory = PathBuf::from("/tmp").join(format!("omega-identity-{user_id}"));
        match fs::create_dir(&lock_directory) {
            Ok(()) => fs::set_permissions(&lock_directory, Permissions::from_mode(0o700))
                .map_err(|_| MutationLockError::PlatformLockUnavailable)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(MutationLockError::PlatformLockUnavailable),
        }

        let directory_metadata = fs::symlink_metadata(&lock_directory)
            .map_err(|_| MutationLockError::PlatformLockUnavailable)?;
        if !directory_metadata.is_dir()
            || directory_metadata.file_type().is_symlink()
            || directory_metadata.uid() != user_id
            || directory_metadata.mode() & 0o077 != 0
        {
            return Err(MutationLockError::PlatformLockUnavailable);
        }

        let lock_path = lock_directory.join(format!("{}.lock", service_hash(service)));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(lock_path)
            .map_err(|_| MutationLockError::PlatformLockUnavailable)?;

        let file_metadata = file
            .metadata()
            .map_err(|_| MutationLockError::PlatformLockUnavailable)?;
        if !file_metadata.is_file()
            || file_metadata.uid() != user_id
            || file_metadata.nlink() != 1
            || file_metadata.mode() & 0o077 != 0
        {
            return Err(MutationLockError::PlatformLockUnavailable);
        }

        loop {
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result == 0 {
                break;
            }
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                return Err(MutationLockError::PlatformLockUnavailable);
            }
        }

        Ok(Self { _file: file })
    }
}

#[cfg(windows)]
struct PlatformMutationGuard {
    mutex_handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl PlatformMutationGuard {
    fn acquire(service: &str) -> Result<Self, MutationLockError> {
        use windows_sys::Win32::{
            Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0},
            Security::SECURITY_ATTRIBUTES,
            System::Threading::{CreateMutexW, INFINITE, WaitForSingleObject},
        };

        let mutex_name = format!("Local\\OmegaIdentity-{}", service_hash(service));
        let wide_name: Vec<u16> = mutex_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mutex_handle = unsafe {
            CreateMutexW(
                std::ptr::null::<SECURITY_ATTRIBUTES>(),
                0,
                wide_name.as_ptr(),
            )
        };
        if mutex_handle.is_null() {
            return Err(MutationLockError::PlatformLockUnavailable);
        }

        let wait_result = unsafe { WaitForSingleObject(mutex_handle, INFINITE) };
        if wait_result != WAIT_OBJECT_0 && wait_result != WAIT_ABANDONED {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(mutex_handle);
            }
            return Err(MutationLockError::PlatformLockUnavailable);
        }

        Ok(Self { mutex_handle })
    }
}

#[cfg(windows)]
impl Drop for PlatformMutationGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.mutex_handle);
            windows_sys::Win32::Foundation::CloseHandle(self.mutex_handle);
        }
    }
}

#[cfg(not(any(unix, windows)))]
struct PlatformMutationGuard;

#[cfg(not(any(unix, windows)))]
impl PlatformMutationGuard {
    fn acquire(_service: &str) -> Result<Self, MutationLockError> {
        Err(MutationLockError::PlatformLockUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use app_identity::AppChannel;

    use super::*;

    #[test]
    fn channel_lock_names_are_deterministic_distinct_and_secret_free() {
        let hashes = AppChannel::ALL
            .map(|channel| service_hash(KeyringLocator::for_channel(channel).service()));
        assert!(hashes.iter().all(|hash| hash.len() == 64));
        assert!(hashes.iter().all(|hash| {
            hash.bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        }));
        for (channel, hash) in AppChannel::ALL.into_iter().zip(hashes.iter()) {
            let service = KeyringLocator::for_channel(channel).service().to_string();
            assert!(!hash.contains(&service));
            assert_eq!(hash, &service_hash(&service));
        }
    }

    #[test]
    fn mutation_guard_can_be_reacquired_after_drop() {
        let locator = KeyringLocator::for_channel(AppChannel::Dev);
        {
            let _guard = IdentityMutationGuard::acquire(&locator).expect("acquire identity lock");
        }
        let _guard = IdentityMutationGuard::acquire(&locator).expect("reacquire identity lock");
    }
}
