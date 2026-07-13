//! Secret storage for provider credentials.
//!
//! API keys never enter `settings.json`.  Windows builds use the per-user
//! Windows Credential Manager; the non-Windows implementation keeps the crate
//! testable without pretending that a plaintext fallback is secure.

pub const GOOGLE_PLACES_TARGET: &str = "GPX Animator/Google Places API Key";

pub fn mask_secret(secret: Option<&str>) -> String {
    match secret.filter(|value| !value.trim().is_empty()) {
        Some(value) if value.len() > 4 => {
            format!("{}••••{}", &value[..2], &value[value.len() - 2..])
        }
        Some(_) => "••••".to_owned(),
        None => String::new(),
    }
}

#[cfg(windows)]
pub fn read_google_places_api_key() -> Result<Option<String>, String> {
    use windows::Win32::Security::Credentials::{
        CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW,
    };
    use windows::core::PCWSTR;
    let target = wide(GOOGLE_PLACES_TARGET);
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: the target is a NUL-terminated UTF-16 string owned for the call;
    // CredReadW allocates the returned struct which is released by CredFree.
    let result = unsafe {
        CredReadW(
            PCWSTR(target.as_ptr()),
            CRED_TYPE_GENERIC,
            None,
            &mut credential,
        )
    };
    if let Err(error) = result {
        // ERROR_NOT_FOUND is the normal first-run state.
        if matches!(error.code().0 as u32, 1168 | 0x8007_0490) {
            return Ok(None);
        }
        return Err(error.to_string());
    }
    if credential.is_null() {
        return Ok(None);
    }
    // SAFETY: the pointer and size are supplied by Credential Manager and are
    // valid until CredFree; API keys are stored as UTF-8 bytes.
    let value = unsafe {
        let value = std::slice::from_raw_parts(
            (*credential).CredentialBlob,
            (*credential).CredentialBlobSize as usize,
        );
        String::from_utf8(value.to_vec()).map_err(|error| error.to_string())
    };
    unsafe { CredFree(credential.cast()) };
    value.map(|value| (!value.trim().is_empty()).then_some(value))
}

#[cfg(not(windows))]
pub fn read_google_places_api_key() -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(windows)]
pub fn write_google_places_api_key(value: &str) -> Result<(), String> {
    use windows::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredWriteW,
    };
    use windows::core::{PCWSTR, PWSTR};
    let value = value.trim();
    if value.is_empty() {
        let target = wide(GOOGLE_PLACES_TARGET);
        // Removing a missing credential is intentionally idempotent.
        // SAFETY: target remains alive for the call.
        let _ = unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) };
        return Ok(());
    }
    let target = wide(GOOGLE_PLACES_TARGET);
    let mut blob = value.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_ptr() as *mut _),
        CredentialBlob: blob.as_mut_ptr(),
        CredentialBlobSize: blob.len() as u32,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..CREDENTIALW::default()
    };
    // SAFETY: all pointers refer to buffers held until CredWriteW returns.
    unsafe { CredWriteW(&credential, 0).map_err(|error| error.to_string()) }
}

#[cfg(not(windows))]
pub fn write_google_places_api_key(_value: &str) -> Result<(), String> {
    Err("Windows Credential Manager is only available on Windows".to_owned())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_without_persisting_secret() {
        assert_eq!(mask_secret(Some("abcd1234")), "ab••••34");
        assert_eq!(mask_secret(Some("abc")), "••••");
        assert!(mask_secret(None).is_empty());
    }

    #[test]
    fn target_is_stable_for_migrations() {
        assert_eq!(GOOGLE_PLACES_TARGET, "GPX Animator/Google Places API Key");
    }
}
