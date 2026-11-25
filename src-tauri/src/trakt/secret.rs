use base64::Engine;
use zeroize::Zeroizing;

const APP_ENTROPY: &[u8] = b"EPGViewer-Trakt-DPAPI-v1";

pub trait SecretStore: Send + Sync {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, String>;
    fn unprotect(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, String>;
}

#[cfg(windows)]
pub struct DpapiStore;

#[cfg(windows)]
impl SecretStore for DpapiStore {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, String> {
        use std::ptr;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        let mut in_buf = plain.to_vec();
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: in_buf.len() as u32,
            pbData: if in_buf.is_empty() {
                ptr::null_mut()
            } else {
                in_buf.as_mut_ptr()
            },
        };

        let mut entropy_buf = APP_ENTROPY.to_vec();
        let entropy_blob = CRYPT_INTEGER_BLOB {
            cbData: entropy_buf.len() as u32,
            pbData: entropy_buf.as_mut_ptr(),
        };

        let mut out_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };

        let ok = unsafe {
            CryptProtectData(
                &in_blob,
                ptr::null(),
                &entropy_blob,
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };

        zeroize::Zeroize::zeroize(&mut in_buf);

        if ok == 0 {
            return Err("DPAPI titkosítás sikertelen (CryptProtectData)".to_string());
        }

        let result = unsafe {
            std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec()
        };
        unsafe {
            LocalFree(out_blob.pbData as _);
        }
        Ok(result)
    }

    fn unprotect(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        use std::ptr;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        if blob.is_empty() {
            return Err("Üres titkosított adat".to_string());
        }

        let mut in_buf = blob.to_vec();
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: in_buf.len() as u32,
            pbData: in_buf.as_mut_ptr(),
        };

        let mut entropy_buf = APP_ENTROPY.to_vec();
        let entropy_blob = CRYPT_INTEGER_BLOB {
            cbData: entropy_buf.len() as u32,
            pbData: entropy_buf.as_mut_ptr(),
        };

        let mut out_blob = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };

        let ok = unsafe {
            CryptUnprotectData(
                &in_blob,
                ptr::null_mut(),
                &entropy_blob,
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out_blob,
            )
        };

        if ok == 0 {
            return Err("DPAPI visszafejtés sikertelen (CryptUnprotectData)".to_string());
        }

        let plain_slice =
            unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) };
        let result = Zeroizing::new(plain_slice.to_vec());
        unsafe {
            ptr::write_bytes(out_blob.pbData, 0, out_blob.cbData as usize);
            LocalFree(out_blob.pbData as _);
        }
        Ok(result)
    }
}

#[cfg(not(windows))]
pub struct StubStore;

#[cfg(not(windows))]
impl SecretStore for StubStore {
    fn protect(&self, _: &[u8]) -> Result<Vec<u8>, String> {
        Err("Token titkosítás csak Windowson támogatott".to_string())
    }

    fn unprotect(&self, _: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
        Err("Token visszafejtés csak Windowson támogatott".to_string())
    }
}

pub fn default_secret_store() -> Box<dyn SecretStore> {
    #[cfg(windows)]
    {
        Box::new(DpapiStore)
    }
    #[cfg(not(windows))]
    {
        Box::new(StubStore)
    }
}

pub fn protect_str(store: &dyn SecretStore, plain: &str) -> Result<String, String> {
    let encrypted = store.protect(plain.as_bytes())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
}

pub fn unprotect_str(store: &dyn SecretStore, b64: &str) -> Result<Zeroizing<String>, String> {
    let blob = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("Base64 dekódolási hiba: {}", e))?;
    let plain_bytes = store.unprotect(&blob)?;
    let s = String::from_utf8(plain_bytes.to_vec())
        .map_err(|e| format!("UTF-8 dekódolási hiba: {}", e))?;
    Ok(Zeroizing::new(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn test_dpapi_roundtrip() {
        let store = DpapiStore;
        let secret = "trakt_access_token_sample_12345";
        let b64 = protect_str(&store, secret).unwrap();
        assert_ne!(b64, secret);
        let decrypted = unprotect_str(&store, &b64).unwrap();
        assert_eq!(decrypted.as_str(), secret);
    }

    #[cfg(windows)]
    #[test]
    fn test_dpapi_corrupted_blob_fails() {
        let store = DpapiStore;
        assert!(store.unprotect(b"not-a-valid-dpapi-blob").is_err());
        assert!(store.unprotect(b"").is_err());
    }
}
