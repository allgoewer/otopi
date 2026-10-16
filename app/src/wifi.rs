//! WiFi handling with `iw`, `ip` and busybox `udhcpc`. The otoscope network is
//! open (no WPA), so no supplicant is needed.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, info, warn};

/// The NE3 uses 2.4 GHz channel 1 only (spec, "network and transport").
const NE3_FREQ_MHZ: &str = "2412";
const DHCP_SCRIPT: &str = "/usr/share/udhcpc/otopi.script";
const STATIC_ADDR: &str = "192.168.169.2/24";

pub struct Wifi {
    iface: String,
    ssid_prefix: String,
    scans: u32,
}

/// Runs a command, kills it after `timeout`. Returns stdout on exit status 0.
fn run(cmd: &str, args: &[&str], timeout: Duration) -> io::Result<String> {
    debug!("run: {cmd} {}", args.join(" "));
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("{cmd}: {e}")))?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err = thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let (out, err) = (
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    );
    match status {
        None => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{cmd} {}: timeout", args.join(" ")),
        )),
        Some(s) if s.success() => Ok(out),
        Some(s) => Err(io::Error::other(format!(
            "{cmd} {}: {s}: {}",
            args.join(" "),
            err.trim()
        ))),
    }
}

/// SSIDs that start with `prefix` (ignoring case) from `iw scan` output,
/// strongest signal first. Devices use e.g. `hndec-1234` or `HNDEC_55-967668`.
fn parse_scan(output: &str, prefix: &str) -> Vec<String> {
    let prefix = prefix.to_lowercase();
    let mut found: Vec<(f32, String)> = Vec::new();
    let mut signal = f32::MIN;
    for line in output.lines() {
        let l = line.trim();
        if line.starts_with("BSS ") {
            signal = f32::MIN;
        } else if let Some(v) = l.strip_prefix("signal:") {
            signal = v
                .trim()
                .trim_end_matches("dBm")
                .trim()
                .parse()
                .unwrap_or(f32::MIN);
        } else if let Some(ssid) = l.strip_prefix("SSID: ") {
            if ssid.to_lowercase().starts_with(&prefix) && !found.iter().any(|(_, s)| s == ssid) {
                found.push((signal, ssid.to_string()));
            }
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    found.into_iter().map(|(_, s)| s).collect()
}

impl Wifi {
    pub fn new(iface: &str, ssid_prefix: &str) -> Wifi {
        Wifi {
            iface: iface.into(),
            ssid_prefix: ssid_prefix.into(),
            scans: 0,
        }
    }

    fn iw(&self, args: &[&str], timeout: Duration) -> io::Result<String> {
        let mut a = vec!["dev", self.iface.as_str()];
        a.extend_from_slice(args);
        run("iw", &a, timeout)
    }

    /// Scans for the otoscope. Ok(None) if the interface is not there yet or nothing was found.
    pub fn find(&mut self) -> io::Result<Option<String>> {
        if !Path::new("/sys/class/net").join(&self.iface).exists() {
            debug!("{} does not exist yet", self.iface);
            return Ok(None);
        }
        run(
            "ip",
            &["link", "set", &self.iface, "up"],
            Duration::from_secs(2),
        )?;
        // Scan channel 1 only (fast); every 5th scan covers all channels in case
        // a device uses another channel.
        self.scans += 1;
        let out = if self.scans % 5 == 0 {
            self.iw(&["scan"], Duration::from_secs(10))?
        } else {
            self.iw(&["scan", "freq", NE3_FREQ_MHZ], Duration::from_secs(5))?
        };
        let found = parse_scan(&out, &self.ssid_prefix);
        if found.len() > 1 {
            info!("found {found:?}, using the strongest");
        }
        Ok(found.into_iter().next())
    }

    /// Associates with `ssid` and configures the address.
    pub fn connect(&mut self, ssid: &str) -> io::Result<()> {
        let _ = self.iw(&["disconnect"], Duration::from_secs(2));
        self.iw(&["connect", ssid], Duration::from_secs(3))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.link_up() {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "association timeout",
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
        info!("associated with {ssid}");
        self.configure_address()
    }

    fn configure_address(&self) -> io::Result<()> {
        let _ = run(
            "ip",
            &["addr", "flush", "dev", &self.iface],
            Duration::from_secs(2),
        );
        let dhcp = run(
            "udhcpc",
            &[
                "-i",
                &self.iface,
                "-n",
                "-q",
                "-f",
                "-t",
                "3",
                "-T",
                "1",
                "-s",
                DHCP_SCRIPT,
            ],
            Duration::from_secs(6),
        );
        match dhcp {
            Ok(_) => info!("address from DHCP"),
            Err(e) => {
                warn!("DHCP failed ({e}); using {STATIC_ADDR}");
                run(
                    "ip",
                    &["addr", "add", STATIC_ADDR, "dev", &self.iface],
                    Duration::from_secs(2),
                )?;
            }
        }
        Ok(())
    }

    pub fn link_up(&self) -> bool {
        self.iw(&["link"], Duration::from_secs(2))
            .is_ok_and(|out| out.starts_with("Connected to"))
    }

    pub fn disconnect(&mut self) {
        let _ = self.iw(&["disconnect"], Duration::from_secs(2));
        let _ = run(
            "ip",
            &["addr", "flush", "dev", &self.iface],
            Duration::from_secs(2),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCAN: &str = "\
BSS 11:22:33:44:55:66(on wlan0)
\tfreq: 2412
\tsignal: -71.00 dBm
\tSSID: hndec-1111
BSS aa:bb:cc:dd:ee:ff(on wlan0)
\tfreq: 2412
\tsignal: -40.00 dBm
\tSSID: hndec-2222
BSS 1c:78:4b:96:76:68(on wlan0)
\tfreq: 2412.0
\tsignal: -26.00 dBm
\tSSID: HNDEC_55-967668
BSS 00:00:00:00:00:01(on wlan0)
\tsignal: -30.00 dBm
\tSSID: HomeNet
BSS 00:00:00:00:00:02(on wlan0)
\tsignal: -50.00 dBm
\tSSID:
";

    #[test]
    fn scan_filters_and_sorts() {
        assert_eq!(
            parse_scan(SCAN, "hndec"),
            vec!["HNDEC_55-967668", "hndec-2222", "hndec-1111"]
        );
        assert!(parse_scan("", "hndec").is_empty());
    }

    #[test]
    fn run_timeout_and_errors() {
        assert_eq!(
            run("echo", &["hi"], Duration::from_secs(2)).unwrap(),
            "hi\n"
        );
        assert!(run("false", &[], Duration::from_secs(2)).is_err());
        let e = run("sleep", &["5"], Duration::from_millis(100)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(run("/nonexistent", &[], Duration::from_secs(1)).is_err());
    }
}
