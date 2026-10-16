mod config;
mod decode;
mod display;
mod jpeg;
mod proto;
mod rotate;
mod session;
mod status;
mod sysinfo;
mod ui;
mod wifi;

use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use log::{error, info, warn};

/// `git describe` of the build (see build.rs).
pub const VERSION: &str = env!("OTOPI_VERSION");

const USAGE: &str = "\
usage: otopi [options]
  --device IP      otoscope address (default 192.168.169.1)
  --bind ADDR      local UDP address (default 0.0.0.0:36000)
  --dump-dir DIR   write each received frame as DIR/NNNNNN.jpg
  --no-wifi        do not manage WiFi (host testing)
  --iface NAME     WiFi interface (default wlan0)
  --ssid-prefix P  otoscope SSID prefix, any case (default hndec)
  --no-display     do not use DRM/KMS output
  --config FILE    settings file (default /boot/otopi.conf)
  --no-rotate      do not keep the image level (overrides the settings file)
  -V, --version    show the version
  -h, --help       show this text
log level: RUST_LOG=debug";

struct Args {
    session: session::Config,
    dump_dir: Option<PathBuf>,
    no_wifi: bool,
    no_display: bool,
    iface: String,
    ssid_prefix: String,
    config: PathBuf,
    no_rotate: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        session: session::Config::default(),
        dump_dir: None,
        no_wifi: false,
        no_display: false,
        iface: "wlan0".into(),
        ssid_prefix: "hndec".into(),
        config: config::DEFAULT_PATH.into(),
        no_rotate: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--device" => {
                args.session.device = value()?.parse().map_err(|e| format!("--device: {e}"))?
            }
            "--bind" => args.session.bind = value()?.parse().map_err(|e| format!("--bind: {e}"))?,
            "--dump-dir" => args.dump_dir = Some(value()?.into()),
            "--no-wifi" => args.no_wifi = true,
            "--no-display" => args.no_display = true,
            "--iface" => args.iface = value()?,
            "--ssid-prefix" => args.ssid_prefix = value()?,
            "--config" => args.config = value()?.into(),
            "--no-rotate" => args.no_rotate = true,
            "-V" | "--version" => {
                println!("otopi {VERSION}");
                std::process::exit(0);
            }
            "-h" | "--help" => return Err(String::new()),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if let Some(dir) = &args.dump_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            error!("cannot create {}: {e}", dir.display());
            return ExitCode::FAILURE;
        }
    }

    info!("otopi {VERSION}");
    let cfg = config::Config::load(&args.config);
    let ui = ui::Ui::new();
    ui.spawn(
        !args.no_display,
        if args.no_rotate {
            rotate::Mode::Off
        } else {
            cfg.rotate
        },
    );

    let mut wifi = (!args.no_wifi).then(|| wifi::Wifi::new(&args.iface, &args.ssid_prefix));
    let mut connected = false;
    // Sessions that ended in silence while the WiFi link stayed up.
    let mut silent_sessions = 0;

    loop {
        if let Some(wifi) = wifi.as_mut() {
            if !connected {
                ui.set_status(ui::Status::Searching);
                let ssid = find_otoscope(wifi);
                ui.set_status(ui::Status::Connecting(ssid.clone()));
                if let Err(e) = wifi.connect(&ssid) {
                    warn!("connect to {ssid}: {e}");
                    wifi.disconnect();
                    thread::sleep(RETRY_DELAY);
                    continue;
                }
                connected = true;
                silent_sessions = 0;
            }
        }

        ui.set_status(ui::Status::WaitingForVideo);
        let mut got_frames = false;
        let result = session::run(&args.session, |frame| {
            if let Some(dir) = &args.dump_dir {
                let path = dir.join(format!("{:06}.jpg", frame.img_number));
                if let Err(e) = std::fs::write(&path, &frame.jpeg) {
                    warn!("cannot write {}: {e}", path.display());
                }
            }
            got_frames = true;
            ui.set_status(ui::Status::Streaming);
            ui.push_frame(frame);
        });
        match result {
            Ok(end) => info!("session ended: {end:?}"),
            Err(e) => error!("session failed: {e}"),
        }
        silent_sessions = if got_frames { 1 } else { silent_sessions + 1 };

        if let Some(wifi) = wifi.as_mut() {
            // The link can stay up while the device restarts its stream; a new
            // session is enough then. Otherwise, or after repeated silence,
            // reconnect from scratch.
            if !wifi.link_up() || silent_sessions >= MAX_SILENT_SESSIONS {
                info!("reconnecting WiFi");
                wifi.disconnect();
                connected = false;
            }
        } else {
            thread::sleep(RETRY_DELAY);
        }
    }
}

const RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_SILENT_SESSIONS: u32 = 2;

/// Scans until an otoscope network shows up.
fn find_otoscope(wifi: &mut wifi::Wifi) -> String {
    loop {
        match wifi.find() {
            Ok(Some(ssid)) => {
                info!("found {ssid}");
                return ssid;
            }
            Ok(None) => {}
            Err(e) => warn!("scan: {e}"),
        }
        thread::sleep(RETRY_DELAY);
    }
}
