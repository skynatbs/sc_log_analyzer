use directories::ProjectDirs;
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const LAST_PATH_FILE: &str = "last_log_path.txt";

pub fn load_last_log_path() -> Option<String> {
    let path = storage_file_path()?;
    let contents = fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn save_last_log_path(path: &Path) -> io::Result<()> {
    let Some(storage_path) = storage_file_path() else {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to resolve settings directory",
        ));
    };

    if let Some(dir) = storage_path.parent() {
        fs::create_dir_all(dir)?;
    }

    let mut file = fs::File::create(storage_path)?;
    file.write_all(path.to_string_lossy().as_bytes())
}

fn storage_file_path() -> Option<PathBuf> {
    project_dirs().map(|dir| dir.join(LAST_PATH_FILE))
}

fn project_dirs() -> Option<PathBuf> {
    if let Some(dirs) = ProjectDirs::from("com", "setscallywag", "sc_log_analyzer") {
        Some(dirs.config_dir().to_path_buf())
    } else {
        fallback_config_dir()
    }
}

fn fallback_config_dir() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|dir| dir.join("sc_log_analyzer"))
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|dir| dir.join(".config").join("sc_log_analyzer"))
        })
}
