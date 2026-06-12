//! Secrets at-rest schützen.
//!
//! Windows: **DPAPI** (`CryptProtectData`, an den Windows-Benutzer gebunden) —
//! die `config.json` enthält dann keine Klartext-Secrets mehr. Andere Plattformen
//! (Dev/Mac): Passthrough. Werte werden mit `dpapi:` + Base64 markiert, damit
//! Alt-Configs (Klartext) beim Laden weiterhin funktionieren.

const PREFIX: &str = "dpapi:";

#[cfg(windows)]
pub fn protect(plain: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};

    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: plain.len() as u32, pbData: plain.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
        let ok = CryptProtectData(&input, null(), null(), null(), null(), 0, &mut out);
        if ok == 0 {
            return plain.to_string(); // Fallback: lieber Klartext als Datenverlust
        }
        let slice = std::slice::from_raw_parts(out.pbData, out.cbData as usize);
        let encoded = STANDARD.encode(slice);
        LocalFree(out.pbData as _);
        format!("{PREFIX}{encoded}")
    }
}

#[cfg(windows)]
pub fn unprotect(stored: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let Some(b64) = stored.strip_prefix(PREFIX) else {
        return stored.to_string(); // Alt-Config / Klartext
    };
    let Ok(mut bytes) = STANDARD.decode(b64) else {
        return String::new();
    };
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_mut_ptr() };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
        let ok = CryptUnprotectData(&input, null_mut(), null(), null(), null(), 0, &mut out);
        if ok == 0 {
            return String::new();
        }
        let slice = std::slice::from_raw_parts(out.pbData, out.cbData as usize);
        let plain = String::from_utf8_lossy(slice).to_string();
        LocalFree(out.pbData as _);
        plain
    }
}

#[cfg(not(windows))]
pub fn protect(plain: &str) -> String {
    plain.to_string()
}

#[cfg(not(windows))]
pub fn unprotect(stored: &str) -> String {
    stored.strip_prefix(PREFIX).map(str::to_string).unwrap_or_else(|| stored.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        // Auf Nicht-Windows Passthrough, auf Windows echtes DPAPI — beidesmal
        // muss unprotect(protect(x)) == x gelten.
        for s in ["wp_ext_deadbeef", "geheim!§$%", ""] {
            assert_eq!(unprotect(&protect(s)), s);
        }
    }

    #[test]
    fn klartext_bleibt_lesbar() {
        // Alt-Config ohne Präfix wird unverändert zurückgegeben.
        assert_eq!(unprotect("legacy-plain"), "legacy-plain");
    }
}
