//! Import existing user data once without overwriting newer settings or deleting originals.
use std::{fs, io, path::Path};

pub fn migrate(identifier: &str) -> io::Result<()> {
    if let Some(root) = dirs::config_dir() {
        migrate_in(&root, identifier)?;
    }
    Ok(())
}

fn migrate_in(root: &Path, identifier: &str) -> io::Result<()> {
    copy_if_missing(
        &root.join("agent-usage-window/settings.json"),
        &root.join("agent-usage-gui/settings.json"),
    )?;
    copy_if_missing(
        &root.join("com.agent-usage-window.app/.window-state.json"),
        &root.join(identifier).join(".window-state.json"),
    )?;
    Ok(())
}

fn copy_if_missing(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.try_exists()? {
        return Ok(());
    }
    let mut input = match fs::File::open(source) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let parent = destination
        .parent()
        .expect("migration destination has a parent");
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    io::copy(&mut input, &mut temporary)?;
    temporary.as_file().sync_all()?;
    // Publish a complete file atomically; another instance may have migrated first.
    match temporary.persist_noclobber(destination) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_both_files_without_touching_originals_or_overwriting_new_data() {
        let root = tempfile::tempdir().unwrap();
        let old_settings = root.path().join("agent-usage-window/settings.json");
        let old_window = root
            .path()
            .join("com.agent-usage-window.app/.window-state.json");
        for path in [&old_settings, &old_window] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        let settings = br#"{"providers":[],"thresholds":{"critical":75},"pinned":false}"#;
        let window = br#"{"main":{"x":30,"y":40,"width":340,"height":260}}"#;
        fs::write(&old_settings, settings).unwrap();
        fs::write(&old_window, window).unwrap();
        migrate_in(root.path(), "com.judealnas.agentusagegui").unwrap();
        let new_settings = root.path().join("agent-usage-gui/settings.json");
        let new_window = root
            .path()
            .join("com.judealnas.agentusagegui/.window-state.json");
        assert_eq!(fs::read(&new_settings).unwrap(), settings);
        assert_eq!(fs::read(&new_window).unwrap(), window);
        assert_eq!(fs::read(old_settings).unwrap(), settings);
        assert_eq!(fs::read(old_window).unwrap(), window);
        fs::write(&new_settings, b"new settings").unwrap();
        fs::write(&new_window, b"new window state").unwrap();
        migrate_in(root.path(), "com.judealnas.agentusagegui").unwrap();
        assert_eq!(fs::read(new_settings).unwrap(), b"new settings");
        assert_eq!(fs::read(new_window).unwrap(), b"new window state");
    }

    #[test]
    fn fresh_install_does_not_create_settings() {
        let root = tempfile::tempdir().unwrap();
        migrate_in(root.path(), "com.judealnas.agentusagegui").unwrap();
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn migration_error_is_reported_and_original_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("old.json");
        fs::write(&source, b"original").unwrap();
        let blocked = root.path().join("blocked");
        fs::write(&blocked, b"not a directory").unwrap();
        assert!(copy_if_missing(&source, &blocked.join("new.json")).is_err());
        assert_eq!(fs::read(source).unwrap(), b"original");
    }
}
