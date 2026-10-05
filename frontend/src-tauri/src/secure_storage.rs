//! At-rest protection for secrets (provider API keys) stored in the local SQLite
//! `settings`/`transcript_settings` tables.
//!
//! Values are run through [`protect`] before being written to disk and [`unprotect`]
//! when read back. Protected values carry a `dpapi:v1:` prefix; anything without that
//! prefix is treated as a legacy plaintext value (saved before this file existed, or
//! saved on a platform where protection isn't implemented) and is upgraded to protected
//! form automatically the next time it's saved — no separate migration step is needed.
//!
//! ponytail: only Windows (DPAPI, via `CryptProtectData`/`CryptUnprotectData`) is
//! implemented. macOS/Linux fall back to plaintext, matching prior behavior on those
//! platforms — never worse than before this change. Upgrade path: route all three
//! platforms through the `keyring` crate (Credential Manager / Keychain / libsecret)
//! once a build environment is available to verify that added dependency compiles
//! cleanly on every target.

const PROTECTED_PREFIX: &str = "dpapi:v1:";

/// Encrypts `plaintext` for storage. Returns a prefixed, hex-encoded ciphertext where OS
/// protection is implemented; returns `plaintext` unchanged where it isn't (or if
/// protection fails), which is never worse than the prior plaintext-only behavior.
pub fn protect(plaintext: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        if let Some(encrypted) = windows_dpapi::protect(plaintext.as_bytes()) {
            return format!("{PROTECTED_PREFIX}{}", to_hex(&encrypted));
        }
    }
    plaintext.to_string()
}

/// Reverses [`protect`]. A value without the protected prefix is assumed to be legacy
/// plaintext and is returned as-is. A protected value that fails to decrypt (e.g. the
/// SQLite file was copied to a different Windows user profile) fails closed to an empty
/// string rather than returning unusable ciphertext as if it were a real key.
pub fn unprotect(stored: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        if let Some(hex_ciphertext) = stored.strip_prefix(PROTECTED_PREFIX) {
            if let Some(bytes) = from_hex(hex_ciphertext) {
                if let Some(decrypted) = windows_dpapi::unprotect(&bytes) {
                    if let Ok(text) = String::from_utf8(decrypted) {
                        return text;
                    }
                }
            }
            return String::new();
        }
    }
    stored.to_string()
}

#[cfg(target_os = "windows")]
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{:02x}", b);
    }
    out
}

#[cfg(target_os = "windows")]
fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(target_os = "windows")]
mod windows_dpapi {
    //! Minimal FFI binding to the Windows Data Protection API (DPAPI). Encrypts/decrypts
    //! bytes bound to the current Windows user account, so the ciphertext stored in the
    //! app's SQLite file is unreadable outside this user profile even if that file is
    //! copied elsewhere.
    use std::os::raw::c_void;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            p_data_in: *mut DataBlob,
            sz_data_descr: *const u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *const c_void,
            p_prompt_struct: *const c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            p_data_in: *mut DataBlob,
            pp_sz_data_descr: *mut *mut u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *const c_void,
            p_prompt_struct: *const c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
    }

    // Suppress any OS credential-prompt UI; this runs in a background service context.
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    pub fn protect(plaintext: &[u8]) -> Option<Vec<u8>> {
        run(plaintext, true)
    }

    pub fn unprotect(ciphertext: &[u8]) -> Option<Vec<u8>> {
        run(ciphertext, false)
    }

    fn run(input_bytes: &[u8], encrypt: bool) -> Option<Vec<u8>> {
        if input_bytes.is_empty() {
            return Some(Vec::new());
        }
        unsafe {
            // DPAPI never writes through pDataIn; the non-const signature is a Win32 API
            // quirk, not an indication the input buffer is mutated.
            let mut input = DataBlob {
                cb_data: input_bytes.len() as u32,
                pb_data: input_bytes.as_ptr() as *mut u8,
            };
            let mut output = DataBlob {
                cb_data: 0,
                pb_data: std::ptr::null_mut(),
            };

            let ok = if encrypt {
                CryptProtectData(
                    &mut input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &mut input,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            };

            if ok == 0 || output.pb_data.is_null() {
                return None;
            }

            let result =
                std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec();
            LocalFree(output.pb_data as *mut c_void);
            Some(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protect_unprotect_roundtrip() {
        let secret = "sk-test-1234567890";
        let protected = protect(secret);

        #[cfg(target_os = "windows")]
        {
            assert!(protected.starts_with(PROTECTED_PREFIX));
            assert_ne!(protected, secret);
        }

        assert_eq!(unprotect(&protected), secret);
    }

    #[test]
    fn unprotect_passes_through_legacy_plaintext() {
        assert_eq!(unprotect("sk-legacy-plaintext"), "sk-legacy-plaintext");
    }

    #[test]
    fn empty_string_roundtrips() {
        assert_eq!(unprotect(&protect("")), "");
    }
}
