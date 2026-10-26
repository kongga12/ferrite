//! 연결 비밀번호 보관. 앱은 OS 키체인(Windows 자격 증명 관리자, macOS 키체인, Linux Secret Service)을
//! 쓰고, 시험은 메모리 저장소를 쓴다(사용자 키체인을 건드리지 않게).

use std::collections::HashMap;
use std::sync::Mutex;

/// 연결 고유 값(`ConnectionConfig::id`)으로 비밀번호를 찾는다.
pub trait SecretStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>, String>;
    fn set(&self, id: &str, password: &str) -> Result<(), String>;
    /// 없으면 아무 일도 없다.
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// 키체인 항목의 서비스 이름.
const SERVICE: &str = "ferrite-db";

pub struct Keychain;

impl Keychain {
    fn entry(id: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, id).map_err(|e| format!("OS 키체인을 쓸 수 없습니다: {e}"))
    }
}

impl SecretStore for Keychain {
    fn get(&self, id: &str) -> Result<Option<String>, String> {
        match Self::entry(id)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("비밀번호를 읽지 못했습니다(OS 키체인): {e}")),
        }
    }

    fn set(&self, id: &str, password: &str) -> Result<(), String> {
        Self::entry(id)?
            .set_password(password)
            .map_err(|e| format!("비밀번호를 저장하지 못했습니다(OS 키체인): {e}"))
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        match Self::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("비밀번호를 지우지 못했습니다(OS 키체인): {e}")),
        }
    }
}

/// 프로세스 안에만 두는 저장소(시험용).
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, id: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().map_err(|e| e.to_string())?.get(id).cloned())
    }

    fn set(&self, id: &str, password: &str) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .insert(id.to_owned(), password.to_owned());
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 메모리_저장소는_쓰고_읽고_지운다() {
        let store = MemoryStore::default();
        assert_eq!(store.get("a").unwrap(), None);
        store.set("a", "비밀").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("비밀"));
        store.delete("a").unwrap();
        store.delete("a").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
    }
}
