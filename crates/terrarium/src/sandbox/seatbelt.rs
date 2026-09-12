use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

unsafe extern "C" {
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// Applies an SBPL sandbox policy to the **current process** via `sandbox_init(3)`.
///
/// Intended to be called inside a `pre_exec` closure (post-fork, pre-exec), where the
/// process is single-threaded and it is safe to call this C function.
///
/// # Safety
/// The caller must ensure this is invoked in a post-fork, pre-exec context.
/// Calling it on a running multi-threaded process is unsafe.
pub unsafe fn apply(sbpl: &str) -> std::io::Result<()> {
    let profile = CString::new(sbpl).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("SBPL contains null byte: {e}"),
        )
    })?;

    let mut errorbuf: *mut c_char = std::ptr::null_mut();

    // flags = 0 → treat profile as SBPL content (not a named profile)
    let ret = unsafe { sandbox_init(profile.as_ptr(), 0, &mut errorbuf) };

    if ret != 0 {
        let msg = if errorbuf.is_null() {
            "sandbox_init failed (no error message)".to_string()
        } else {
            let s = unsafe { CStr::from_ptr(errorbuf) }
                .to_string_lossy()
                .into_owned();
            unsafe { sandbox_free_error(errorbuf) };
            s
        };
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            msg,
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_minimal_allow_all_profile() {
        // Applying an allow-all profile to a fork is only safe in pre_exec;
        // here we test that the SBPL string doesn't trip up the C API itself.
        // We use a trivially permissive profile.
        let sbpl = "(version 1)\n(allow default)\n";
        // SAFETY: This test runs in a single-threaded test context; no other
        // threads are touching sandbox state. The profile allows everything so
        // it won't break the test process.
        let result = unsafe { apply(sbpl) };
        match result {
            Ok(()) => {}
            Err(err) => {
                assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
                assert!(
                    err.to_string().contains("Operation not permitted"),
                    "unexpected sandbox_init error: {err}"
                );
            }
        }
    }

    #[test]
    fn apply_empty_string_returns_error() {
        // An empty SBPL string is invalid; sandbox_init should return non-zero.
        let result = unsafe { apply("") };
        assert!(result.is_err());
    }
}
