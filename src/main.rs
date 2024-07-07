use anstyle::{AnsiColor, Color, Style};
use anyhow::{bail, Result};
use clap::{ArgAction, Parser};
use env_logger::Env;
use fd_lock::RwLock;
use log::{debug, error, info};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::Path;
use std::process;

const BOLD: Style = Style::new().bold();
const CYAN: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan)));
const GREEN: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)));

#[derive(Debug, clap::Parser)]
pub struct Args {
    /// The package that should be built
    name: Option<String>,
    /// Skip fetching source code
    #[arg(long)]
    skip_fetch: bool,
    /// Skip unpacking the source code
    #[arg(long)]
    skip_extract: bool,
    /// Skip running the build
    #[arg(long)]
    skip_build: bool,
    /// Increase logging output (can be used multiple times)
    #[arg(short, long, global = true, action(ArgAction::Count))]
    verbose: u8,
}

#[derive(Debug, serde::Deserialize)]
pub struct Config {
    meta: Meta,
    #[serde(default)]
    checksums: Vec<Checksum>,
    build: Build,
}

#[derive(Debug, serde::Deserialize)]
pub struct Meta {
    repo: String,
    version: String,
    #[serde(default)]
    suffix: String,
    checkout: Option<String>,
    #[serde(default)]
    patches: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Checksum {
    path: String,
    checksum: String,
}

impl Checksum {
    fn verify<P: AsRef<Path>>(&self, path: P) -> Result<Option<Option<String>>> {
        let path = path.as_ref().join(&self.path);
        let mut sha256 = Sha256::new();
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        io::copy(&mut file, &mut sha256)?;
        let hash = sha256.finalize();
        let calculated = format!("sha256:{}", hex::encode(hash));
        if calculated != self.checksum {
            Ok(Some(Some(calculated)))
        } else {
            Ok(Some(None))
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct Build {
    cmd: String,
}

fn prepare_source(name: &str, config: &Config) -> Result<()> {
    let path = format!("sources/{name}");
    if !fs::metadata(&path).is_ok() {
        process::Command::new("git")
            .args(["init", "--bare", "-qb", "main", &path])
            .status()?;

        process::Command::new("git")
            .args(["-C", &path, "remote", "add", "origin", &config.meta.repo])
            .status()?;
    }

    process::Command::new("git")
        .args(["-C", &path, "fetch", "origin"])
        .status()?;

    Ok(())
}

fn extract_source(name: &str, config: &mut Config) -> Result<()> {
    let source_path = format!("sources/{name}");
    let build_path = format!("build/{name}");

    let checkout = if let Some(checkout) = config.meta.checkout.take() {
        checkout
    } else {
        format!("v{}", config.meta.version)
    };

    let mut child = process::Command::new("git")
        .args([
            "-C",
            &source_path,
            "-c",
            "core.abbrev=no",
            "archive",
            "--format",
            "tar",
            &checkout,
        ])
        .stdout(process::Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let mut archive = tar::Archive::new(stdout);
    archive.unpack(&build_path)?;

    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let log_level = match args.verbose {
        0 => "apt_vulns_xyz=info",
        1 => "apt_vulns_xyz=debug",
        _ => "debug",
    };
    env_logger::init_from_env(Env::default().default_filter_or(log_level));

    if let Some(name) = args.name {
        let path = format!("pkgs/{name}/build.toml");
        let buf = fs::read_to_string(&path)?;
        let mut config = toml::from_str::<Config>(&buf)?;

        info!("Preparing git checkout");
        if !args.skip_fetch {
            prepare_source(&name, &mut config)?;
        }

        let build_path = format!("build/{name}");
        fs::create_dir_all(&build_path)?;
        let mut build_dir = RwLock::new(fs::File::open("build/")?);
        info!("Waiting for build handle");
        let _lock = build_dir.write()?;
        info!("Got build handle");

        if !args.skip_extract {
            fs::remove_dir_all(&build_path)?;
            extract_source(&name, &mut config)?;

            for patch in config.meta.patches {
                info!("Applying patch: {patch:?}");
                process::Command::new("patch")
                    .args([
                        "--forward",
                        "--strip=1",
                        "-i",
                        &format!("../../pkgs/{name}/{patch}"),
                    ])
                    .current_dir(fs::canonicalize(&build_path)?)
                    .status()?;
            }
        }

        if !args.skip_build {
            info!("Starting build");
            debug!("Executing command: {:?}", config.build.cmd);
            process::Command::new("repro-env")
                .args([
                    "build",
                    "--env",
                    &format!("DEB_VERSION={}{}", config.meta.version, config.meta.suffix),
                    "-f",
                    &format!("../../pkgs/{name}/repro-env.lock"),
                    "--",
                    "sh",
                    "-c",
                    &config.build.cmd,
                ])
                .current_dir(fs::canonicalize(&build_path)?)
                .status()?;
        }

        let mut checksum_mismatches = false;
        for rule in config.checksums {
            info!("Checking expected checksum for {:?}", rule.path);
            match rule.verify(&build_path)? {
                Some(Some(calculated)) => {
                    error!(
                        "Compiled artifact ({}) does not match expected checksum ({})",
                        calculated, rule.checksum
                    );
                    checksum_mismatches = true;
                }
                Some(None) => (),
                None => {
                    error!("Missing compiled artifact {:?}", rule.path);
                    checksum_mismatches = true;
                }
            }
        }

        if checksum_mismatches {
            bail!("Some artifact checksums mismatched");
        }
    } else {
        let mut entries = fs::read_dir("pkgs")?
            .map(|x| x.map_err(anyhow::Error::from))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|de| de.file_name());

        for entry in entries {
            let path = entry.path();
            let Some(name) = path.file_name() else {
                continue;
            };
            let Some(name) = name.to_str() else { continue };

            let path = path.join("build.toml");
            let buf = fs::read_to_string(&path)?;
            let config = toml::from_str::<Config>(&buf)?;

            let build_path = format!("build/{name}");

            let mut built = None;
            for rule in config.checksums {
                if built == Some(false) {
                    continue;
                };

                built = match rule.verify(&build_path)? {
                    Some(Some(_calculated)) => Some(false),
                    Some(None) => Some(true),
                    None => Some(false),
                };
            }

            println!(
                "{GREEN}{:>7}{GREEN:#} {BOLD}{:>18}{BOLD:#} {CYAN}{:>8}{}{CYAN:#}: {}",
                if built == Some(true) { "[built]" } else { "" },
                name,
                config.meta.version,
                config.meta.suffix,
                config.meta.repo
            );
        }
    }

    Ok(())
}
