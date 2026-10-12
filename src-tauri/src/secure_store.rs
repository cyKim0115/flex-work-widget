//! DPAPI(CurrentUser)로 봉인한 파일 저장소.
//!
//! 봉인본은 같은 Windows 계정에서만 풀린다. 다른 계정·다른 PC로 복사한 파일이나 손상된 파일은
//! 읽기에서 `None`이 되고, 호출 쪽은 "로그인 필요"로 처리한다. 파일 교체는 쓰기 경로에서만 한다.
//! 이 모듈은 서비스와 무관하다(claude-usage-widget 등이 그대로 복사해 쓴다).

use std::fs;
use std::path::Path;

/// `plain`을 봉인해 `path`에 원자적으로 쓴다(임시 파일 + rename).
pub fn write_sealed(path: &Path, plain: &[u8]) -> Result<(), String> {
    let sealed = protect(plain)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("저장 폴더 생성 실패: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &sealed).map_err(|e| format!("임시 파일 쓰기 실패: {e}"))?;
    // std::fs::rename 은 Windows에서 MOVEFILE_REPLACE_EXISTING 이라 기존 파일을 덮는다.
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("저장 파일 교체 실패: {e}")
    })
}

/// 봉인 파일을 읽어 푼다. 없거나 풀 수 없으면 `None`.
pub fn read_sealed(path: &Path) -> Option<Vec<u8>> {
    let sealed = fs::read(path).ok()?;
    unprotect(&sealed).ok()
}

/// 봉인 파일을 지운다. 없으면 성공으로 본다.
pub fn delete(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("저장 파일 삭제 실패: {e}")),
    }
}

#[cfg(windows)]
mod dpapi {
    use std::ptr;

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// DPAPI가 LocalAlloc 으로 돌려준 버퍼를 복사하고 해제한다.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        bytes
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(plain);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        // SAFETY: input 은 plain 을 빌린 채로 호출 동안만 쓰이고, out 은 성공 시 DPAPI가 채운다.
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(format!("DPAPI 봉인 실패: {}", std::io::Error::last_os_error()));
        }
        Ok(unsafe { take(out) })
    }

    pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(sealed);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        // SAFETY: protect 와 같다.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err("DPAPI 해제 실패".into());
        }
        Ok(unsafe { take(out) })
    }
}

#[cfg(windows)]
use dpapi::{protect, unprotect};

#[cfg(not(windows))]
fn protect(_plain: &[u8]) -> Result<Vec<u8>, String> {
    Err("봉인 저장은 Windows에서만 지원합니다".into())
}

#[cfg(not(windows))]
fn unprotect(_sealed: &[u8]) -> Result<Vec<u8>, String> {
    Err("봉인 저장은 Windows에서만 지원합니다".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("secure-store-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn dpapi_round_trip() {
        let plain = b"{\"cookies\":[]}";
        let sealed = protect(plain).unwrap();
        assert_ne!(sealed.as_slice(), plain.as_slice());
        assert_eq!(unprotect(&sealed).unwrap(), plain);
    }

    #[test]
    fn file_round_trip_and_atomic_replace() {
        let path = temp_path("rt.dat");
        write_sealed(&path, b"first").unwrap();
        write_sealed(&path, b"second").unwrap();
        assert_eq!(read_sealed(&path).unwrap(), b"second");
        assert!(!path.with_extension("tmp").exists());
        delete(&path).unwrap();
        assert!(read_sealed(&path).is_none());
        delete(&path).unwrap();
    }

    #[test]
    fn corrupt_file_reads_as_none() {
        let path = temp_path("bad.dat");
        fs::write(&path, b"not a dpapi blob").unwrap();
        assert!(read_sealed(&path).is_none());
        delete(&path).unwrap();
    }
}
