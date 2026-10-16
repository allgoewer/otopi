//! CPU usage, SoC temperature and version for the corner of the video.

use std::fs;
use std::time::{Duration, Instant};

const UPDATE: Duration = Duration::from_secs(1);
const THERMAL: &str = "/sys/class/thermal/thermal_zone0/temp";

pub struct SysInfo {
    /// (total, idle) jiffies of the last /proc/stat sample.
    last_stat: Option<(u64, u64)>,
    last_update: Option<Instant>,
    text: String,
}

/// (total, idle) from the aggregate "cpu" line of /proc/stat; idle includes iowait.
fn parse_stat(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|f| f.parse().ok())
        .collect::<Option<_>>()?;
    let idle = v.get(3)? + v.get(4).unwrap_or(&0);
    // guest times are already counted in user and nice
    let total = v.iter().take(8).sum();
    Some((total, idle))
}

/// CPU usage in percent between two samples.
fn usage(prev: (u64, u64), now: (u64, u64)) -> Option<u32> {
    let total = now.0.checked_sub(prev.0)?;
    let idle = now.1.checked_sub(prev.1)?;
    (total > 0).then(|| ((total - idle.min(total)) * 100 / total) as u32)
}

/// Formats the corner text; missing values are shown as "--".
fn format(cpu: Option<u32>, millideg: Option<i64>, version: &str) -> String {
    let cpu = cpu.map_or("--".into(), |c| c.to_string());
    let temp = millideg.map_or("--".into(), |t| (t / 1000).to_string());
    format!("CPU {cpu:>3}%  {temp}\u{b0}C  {version}")
}

impl SysInfo {
    pub fn new() -> SysInfo {
        SysInfo {
            last_stat: None,
            last_update: None,
            text: String::new(),
        }
    }

    /// The current text, re-read at most once per second.
    pub fn text(&mut self) -> &str {
        if self.last_update.is_none_or(|t| t.elapsed() >= UPDATE) {
            self.last_update = Some(Instant::now());
            let stat = fs::read_to_string("/proc/stat")
                .ok()
                .and_then(|s| parse_stat(&s));
            let cpu = self.last_stat.zip(stat).and_then(|(p, n)| usage(p, n));
            if stat.is_some() {
                self.last_stat = stat;
            }
            let temp = fs::read_to_string(THERMAL)
                .ok()
                .and_then(|s| s.trim().parse().ok());
            self.text = format(cpu, temp, crate::VERSION);
        }
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_usage_from_proc_stat() {
        let a = parse_stat("cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 100 0 50 800 50 0 0 0 0 0\n");
        let b = parse_stat("cpu  130 0 70 840 60 0 0 0 0 0\n");
        assert_eq!(a, Some((1000, 850)));
        assert_eq!(usage(a.unwrap(), b.unwrap()), Some(50));
        assert_eq!(usage(b.unwrap(), b.unwrap()), None, "no time passed");
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn text_format() {
        assert_eq!(
            format(Some(7), Some(48_312), "v0.1.0"),
            "CPU   7%  48\u{b0}C  v0.1.0"
        );
        assert_eq!(
            format(None, None, "abc1234"),
            "CPU  --%  --\u{b0}C  abc1234"
        );
        assert!(SysInfo::new().text().ends_with(crate::VERSION));
    }
}
