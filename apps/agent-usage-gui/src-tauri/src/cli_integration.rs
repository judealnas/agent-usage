//! Current-user CLI registration. Never executes or removes discovered CLIs.
use serde::Serialize;

#[derive(Serialize)]
pub struct CliStatus {
    pub supported: bool,
    pub available: bool,
    pub enabled: bool,
    pub managed: bool,
    pub conflicts: Vec<String>,
}

#[cfg(windows)]
mod platform {
    use super::CliStatus;
    use std::{io, path::PathBuf, sync::Mutex};
    use windows_sys::Win32::{
        System::Environment::ExpandEnvironmentStringsW, UI::WindowsAndMessaging::*,
    };
    use winreg::{enums::*, RegKey, RegValue};

    const KEY: &str = r"Software\AgentUsage\CliIntegration";
    static LOCK: Mutex<()> = Mutex::new(());

    fn directory() -> io::Result<PathBuf> {
        std::env::current_exe()?
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("Missing installation directory"))
    }

    fn expand(value: &str) -> String {
        let input: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let length = ExpandEnvironmentStringsW(input.as_ptr(), std::ptr::null_mut(), 0);
            if length == 0 {
                return value.into();
            }
            let mut output = vec![0; length as usize];
            let written = ExpandEnvironmentStringsW(input.as_ptr(), output.as_mut_ptr(), length);
            if written == 0 || written > length {
                return value.into();
            }
            String::from_utf16_lossy(&output[..written as usize - 1])
        }
    }

    fn normalized(value: &str) -> String {
        let expanded = expand(value.trim().trim_matches('"'));
        let path = PathBuf::from(&expanded);
        path.canonicalize()
            .unwrap_or(path)
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    }

    fn read_path(key: &RegKey) -> io::Result<(String, RegType)> {
        match key.get_raw_value("Path") {
            Ok(raw) => {
                if raw.vtype != REG_SZ && raw.vtype != REG_EXPAND_SZ {
                    return Err(io::Error::other("PATH is not a string"));
                }
                let text = winreg::types::FromRegValue::from_reg_value(&raw)?;
                Ok((text, raw.vtype))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((String::new(), REG_EXPAND_SZ)),
            Err(e) => Err(e),
        }
    }

    fn write_path(key: &RegKey, path: &str, kind: RegType) -> io::Result<()> {
        let bytes = path
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        key.set_raw_value("Path", &RegValue { bytes, vtype: kind })?;
        let environment: Vec<u16> = "Environment".encode_utf16().chain(Some(0)).collect();
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                environment.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                1000,
                std::ptr::null_mut(),
            );
        }
        Ok(())
    }

    fn optional_string(key: &RegKey, name: &str) -> io::Result<String> {
        match key.get_value(name) {
            Ok(value) => Ok(value),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    fn status_inner() -> io::Result<CliStatus> {
        let directory = directory()?;
        let own = normalized(&directory.to_string_lossy());
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let user = hkcu.open_subkey("Environment")?;
        let machine = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment")?;
        let (user_path, _) = read_path(&user)?;
        let (machine_path, _) = read_path(&machine)?;
        // Read persistent PATH so Check again sees changes made since GUI startup.
        let (enabled, conflicts) =
            scan_paths(&user_path, &machine_path, &own, |file| file.is_file());
        let owned = match hkcu.open_subkey(KEY) {
            Ok(key) => optional_string(&key, "OwnedEntry")?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        Ok(CliStatus {
            supported: true,
            available: directory.join("agent-usage-cli.exe").is_file(),
            enabled,
            managed: !owned.is_empty() && normalized(&owned) == own,
            conflicts,
        })
    }

    fn scan_paths(
        user_path: &str,
        machine_path: &str,
        own: &str,
        exists: impl Fn(&std::path::Path) -> bool,
    ) -> (bool, Vec<String>) {
        let mut enabled = false;
        let mut conflicts = Vec::new();
        for entry in machine_path
            .split(';')
            .chain(user_path.split(';'))
            .filter(|s| !s.trim().is_empty())
        {
            let directory = PathBuf::from(expand(entry.trim().trim_matches('"')));
            for extension in ["exe", "cmd", "bat", "com"] {
                let file = directory.join(format!("agent-usage-cli.{extension}"));
                if !exists(&file) {
                    continue;
                }
                if normalized(&directory.to_string_lossy()) == own && extension == "exe" {
                    enabled = true;
                } else {
                    let path = file.to_string_lossy().into_owned();
                    if !conflicts
                        .iter()
                        .any(|existing: &String| normalized(existing) == normalized(&path))
                    {
                        conflicts.push(path);
                    }
                }
            }
        }
        (enabled, conflicts)
    }

    pub fn status() -> Result<CliStatus, String> {
        let _guard = LOCK.lock().map_err(|e| e.to_string())?;
        status_inner().map_err(|e| e.to_string())
    }

    pub fn set_enabled(enabled: bool, preserve_preference: bool) -> Result<CliStatus, String> {
        let _guard = LOCK.lock().map_err(|e| e.to_string())?;
        change(enabled, preserve_preference).map_err(|e| e.to_string())?;
        status_inner().map_err(|e| e.to_string())
    }

    fn change(enabled: bool, preserve_preference: bool) -> io::Result<()> {
        let status = status_inner()?;
        if enabled && (!status.available || !status.conflicts.is_empty()) {
            return Err(io::Error::other("Cannot enable the bundled CLI: it is missing or an independent CLI is on PATH. Check again in Settings."));
        }
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(KEY)?;
        let user = hkcu.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;
        let (path, kind) = read_path(&user)?;
        let own = directory()?.to_string_lossy().into_owned();
        let owned = optional_string(&key, "OwnedEntry")?;
        if enabled && !status.enabled {
            if !owned.is_empty() && normalized(&owned) != normalized(&own) {
                return Err(io::Error::other(
                    "Another GUI installation owns the CLI PATH entry. Disable it there first.",
                ));
            }
            // Record ownership first, so interrupted writes remain recoverable.
            key.set_value("OwnedEntry", &own)?;
            let next = append_entry(&path, &own);
            if let Err(error) = write_path(&user, &next, kind) {
                key.set_value("OwnedEntry", &owned)?;
                return Err(error);
            }
        } else if !enabled && !owned.is_empty() && normalized(&owned) == normalized(&own) {
            if let Some(next) = remove_owned_entry(&path, &owned) {
                write_path(&user, &next, kind)?;
            }
            key.delete_value("OwnedEntry")?;
        }
        if !preserve_preference {
            key.set_value("Enabled", &u32::from(enabled))?;
        }
        Ok(())
    }

    pub fn restore() -> Result<(), String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let enabled = hkcu
            .open_subkey(KEY)
            .ok()
            .and_then(|key| key.get_value::<u32, _>("Enabled").ok())
            .unwrap_or(0);
        if enabled != 0 {
            set_enabled(true, true)?;
        }
        Ok(())
    }

    // Exact, single-token removal: never delete a substring or claim an existing entry.
    fn append_entry(path: &str, entry: &str) -> String {
        if path.is_empty() {
            entry.to_owned()
        } else {
            format!("{path};{entry}")
        }
    }

    fn remove_owned_entry(path: &str, owned: &str) -> Option<String> {
        let mut entries: Vec<_> = path.split(';').collect();
        let index = entries.iter().rposition(|entry| *entry == owned)?;
        entries.remove(index);
        Some(entries.join(";"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn discovery_distinguishes_bundled_cli_and_independent_wrappers() {
            let own = normalized(r"C:\Agent Usage");
            let files = [
                r"C:\Agent Usage\agent-usage-cli.exe",
                r"C:\Tools\agent-usage-cli.cmd",
                r"C:\Cargo\bin\agent-usage-cli.exe",
            ];
            let (enabled, conflicts) = scan_paths(
                r#""c:\agent usage\";C:\Cargo\bin;C:\Missing"#,
                r"C:\Tools;C:\Cargo\bin",
                &own,
                |file| {
                    files
                        .iter()
                        .any(|f| normalized(f) == normalized(&file.to_string_lossy()))
                },
            );
            assert!(enabled);
            assert_eq!(conflicts.len(), 2);
            assert!(conflicts.iter().any(|p| p.ends_with(".cmd")));
            let (enabled, conflicts) = scan_paths(r"C:\Missing", "", &own, |_| false);
            assert!(!enabled);
            assert!(conflicts.is_empty());
        }

        #[test]
        fn path_roundtrip_preserves_empty_entries_variables_and_quotes() {
            for path in ["", r"C:\Tools", r#"%CARGO_HOME%\bin;;"C:\Other Tools";"#] {
                assert_eq!(
                    remove_owned_entry(&append_entry(path, r"C:\Agent Usage"), r"C:\Agent Usage"),
                    Some(path.into())
                );
            }
        }
        #[test]
        fn removal_is_exact_and_removes_only_one_entry() {
            assert_eq!(
                remove_owned_entry(r"C:\App-extra;C:\App;C:\App", r"C:\App"),
                Some(r"C:\App-extra;C:\App".into())
            );
            assert_eq!(remove_owned_entry(r"C:\App-extra", r"C:\App"), None);
            assert_eq!(remove_owned_entry(r"c:\app", r"C:\App"), None);
        }
    }
}

#[cfg(windows)]
pub use platform::{restore, set_enabled, status};

#[cfg(not(windows))]
pub fn status() -> Result<CliStatus, String> {
    Ok(CliStatus {
        supported: false,
        available: false,
        enabled: false,
        managed: false,
        conflicts: vec![],
    })
}
#[cfg(not(windows))]
pub fn set_enabled(_: bool, _: bool) -> Result<CliStatus, String> {
    Err("CLI registration is currently supported only on Windows".into())
}
#[cfg(not(windows))]
pub fn restore() -> Result<(), String> {
    Ok(())
}
