//! User settings from a small `key = value` file on the boot partition.

use std::io;
use std::path::Path;

use log::{info, warn};

use crate::rotate::Mode;

pub const DEFAULT_PATH: &str = "/boot/otopi.conf";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Keep the image level with the tilt sensor.
    pub rotate: Mode,
}

impl Default for Config {
    fn default() -> Self {
        Config { rotate: Mode::Snap }
    }
}

fn parse_mode(v: &str) -> Option<Mode> {
    match v.to_ascii_lowercase().as_str() {
        "snap" => Some(Mode::Snap),
        "free" | "on" | "yes" | "true" | "1" => Some(Mode::Free),
        "off" | "no" | "false" | "0" => Some(Mode::Off),
        _ => None,
    }
}

impl Config {
    /// Parses the file content. Bad lines are logged and ignored.
    pub fn parse(text: &str) -> Config {
        let mut cfg = Config::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap().trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                warn!("config line {}: expected key = value", n + 1);
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "rotate" => match parse_mode(value) {
                    Some(m) => cfg.rotate = m,
                    None => warn!(
                        "config line {}: rotate: expected snap, free or off, not {value:?}",
                        n + 1
                    ),
                },
                _ => warn!("config line {}: unknown key {key:?}", n + 1),
            }
        }
        cfg
    }

    /// Reads the file. A missing file gives the defaults.
    pub fn load(path: &Path) -> Config {
        let cfg = match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                info!("{} not found, using defaults", path.display());
                Config::default()
            }
            Err(e) => {
                warn!("{}: {e}; using defaults", path.display());
                Config::default()
            }
        };
        info!("config: {cfg:?}");
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_values() {
        assert_eq!(Config::parse(""), Config::default());
        assert_eq!(Config::default().rotate, Mode::Snap);
        assert_eq!(Config::parse("rotate = off\n").rotate, Mode::Off);
        assert_eq!(Config::parse("rotate = Free").rotate, Mode::Free);
        assert_eq!(Config::parse("rotate = on").rotate, Mode::Free);
        assert_eq!(
            Config::parse("  # comment\n\n rotate=No # trailing\n").rotate,
            Mode::Off
        );
        assert_eq!(
            Config::parse("rotate = off\nrotate = snap").rotate,
            Mode::Snap,
            "last line wins"
        );
    }

    #[test]
    fn bad_lines_keep_defaults() {
        assert_eq!(Config::parse("rotate = maybe").rotate, Mode::Snap);
        assert_eq!(Config::parse("rotate\nfoo = bar").rotate, Mode::Snap);
    }

    #[test]
    fn missing_file() {
        assert_eq!(
            Config::load(Path::new("/nonexistent/otopi.conf")),
            Config::default()
        );
    }
}
