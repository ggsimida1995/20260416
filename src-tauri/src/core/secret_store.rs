use keyring::Entry;

const SERVICE: &str = "com.projectfilecompare.hollysys";
const ACCOUNT_KEY: &str = "default";

fn entry() -> keyring::Result<Entry> {
    Entry::new(SERVICE, ACCOUNT_KEY)
}

pub fn load_password() -> Option<String> {
    let entry = entry().ok()?;
    match entry.get_password() {
        Ok(value) => Some(value),
        Err(keyring::Error::NoEntry) => None,
        Err(error) => {
            eprintln!("[secret_store] load_password failed: {error}");
            None
        }
    }
}

pub fn save_password(password: &str) {
    let Ok(entry) = entry() else {
        eprintln!("[secret_store] entry unavailable, password not stored");
        return;
    };
    let result = if password.is_empty() {
        entry.delete_credential().or_else(|error| match error {
            keyring::Error::NoEntry => Ok(()),
            other => Err(other),
        })
    } else {
        entry.set_password(password)
    };
    if let Err(error) = result {
        eprintln!("[secret_store] save_password failed: {error}");
    }
}
