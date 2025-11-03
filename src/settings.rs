use directories::ProjectDirs;
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const LAST_PATH_FILE: &str = "last_log_path.txt";
const IGNORED_PLAYER_FILE: &str = "ignored_player.txt";

pub fn load_last_log_path() -> Option<String> {
    read_setting(LAST_PATH_FILE).and_then(|contents| {
        let trimmed = contents.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub fn save_last_log_path(path: &Path) -> io::Result<()> {
    let as_str = path.to_string_lossy();
    write_setting(LAST_PATH_FILE, as_str.as_ref())
}

pub fn load_ignored_player() -> Option<String> {
    read_setting(IGNORED_PLAYER_FILE)
}

pub fn save_ignored_player(value: &str) -> io::Result<()> {
    write_setting(IGNORED_PLAYER_FILE, value)
}

fn read_setting(file_name: &str) -> Option<String> {
    let path = storage_file_path(file_name)?;
    let mut contents = fs::read_to_string(path).ok()?;
    while contents.ends_with('\n') || contents.ends_with('\r') {
        contents.pop();
    }
    Some(contents)
}

fn write_setting(file_name: &str, contents: &str) -> io::Result<()> {
    let Some(storage_path) = storage_file_path(file_name) else {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to resolve settings directory",
        ));
    };

    if let Some(dir) = storage_path.parent() {
        fs::create_dir_all(dir)?;
    }

    let mut file = fs::File::create(storage_path)?;
    file.write_all(contents.as_bytes())
}

fn storage_file_path(file_name: &str) -> Option<PathBuf> {
    project_dirs().map(|dir| dir.join(file_name))
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
