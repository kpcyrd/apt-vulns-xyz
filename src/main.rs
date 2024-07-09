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
    /// Increase logging output (can be used multiple times)
    #[arg(short, long, global = true, action(ArgAction::Count))]
    verbose: u8,
    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, clap::Parser)]
pub enum Subcommand {
    /// Build a project into a .deb package
    Build {
        /// The package that should be built
        name: String,
        /// Skip fetching source code
        #[arg(long)]
        skip_fetch: bool,
        /// Skip unpacking the source code
        #[arg(long)]
        skip_extract: bool,
        /// Skip running the build
        #[arg(long)]
        skip_build: bool,
    },
    /// List all configured packages
    List,
    /// Add all files of a package with reprepro
    Include {
        /// The code name of the distribution to add it to (e.g. `stable`)
        distribution: String,
        /// The name of the package which files should be added
        name: String,
        /// Do not actually run reprepro
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Publish the reprepro repository
    Publish {
        /// Do not actually upload, instead do a dry-run
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// The target to upload to
        #[arg(long, default_value = "apt:/var/www/html/")]
        target: String,
    },
}

#[derive(Debug, serde::Deserialize)]
pub struct Config {
    meta: Meta,
    #[serde(default)]
    checksums: Vec<Checksum>,
    build: Build,
}

impl Config {
    pub fn load_from<P: AsRef<Path>>(path: P) -> Result<Self> {
        let buf = fs::read_to_string(&path)?;
        let config = toml::from_str(&buf)?;
        Ok(config)
    }

    pub fn build_status(&self, build_path: &str) -> Result<Option<bool>> {
        let mut built = None;
        for rule in &self.checksums {
            if built == Some(false) {
                continue;
            };

            built = match rule.verify(build_path)? {
                Some(Some(_calculated)) => Some(false),
                Some(None) => Some(true),
                None => Some(false),
            };
        }
        Ok(built)
    }
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

fn status_to_err(label: &str, status: process::ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("Command ({label}) did not complete successfully");
    }
}

fn prepare_source(name: &str, config: &Config) -> Result<()> {
    let path = format!("sources/{name}");
    if fs::metadata(&path).is_err() {
        status_to_err(
            "git",
            process::Command::new("git")
                .args(["init", "--bare", "-qb", "main", &path])
                .status()?,
        )?;

        status_to_err(
            "git",
            process::Command::new("git")
                .args(["-C", &path, "remote", "add", "origin", &config.meta.repo])
                .status()?,
        )?;
    }

    status_to_err(
        "git",
        process::Command::new("git")
            .args(["-C", &path, "fetch", "origin"])
            .status()?,
    )?;

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

    status_to_err("git-archive", child.wait()?)?;

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

    match args.subcommand {
        Subcommand::Build {
            name,
            skip_fetch,
            skip_extract,
            skip_build,
        } => {
            let path = format!("pkgs/{name}/build.toml");
            let buf = fs::read_to_string(path)?;
            let mut config = toml::from_str::<Config>(&buf)?;

            info!("Preparing git checkout");
            if !skip_fetch {
                prepare_source(&name, &config)?;
            }

            let build_path = format!("build/{name}");
            fs::create_dir_all(&build_path)?;
            let mut build_dir = RwLock::new(fs::File::open("build/")?);
            info!("Waiting for build handle");
            let _lock = build_dir.write()?;
            info!("Got build handle");

            if !skip_extract {
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

            if !skip_build {
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
        }
        Subcommand::List => {
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

                let config = Config::load_from(path.join("build.toml"))?;
                let built = config.build_status(&format!("build/{name}"))?;
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
        Subcommand::Include {
            distribution,
            name,
            dry_run,
        } => {
            let config = Config::load_from(format!("pkgs/{name}/build.toml"))?;

            if config.build_status(&format!("build/{name}"))? != Some(true) {
                bail!("Package needs to be built first, missing files or mismatched checksum")
            }

            info!("Dry run mode is enabled");
            for rule in config.checksums {
                let path = format!("build/{name}/{}", rule.path);

                let args = ["includedeb", &distribution, &path];
                info!("Running {args:?}...");
                if dry_run {
                    continue;
                }

                status_to_err(
                    "reprepro",
                    process::Command::new("reprepro").args(args).status()?,
                )?;
            }
        }
        Subcommand::Publish { dry_run, target } => {
            let flags = if dry_run {
                info!("Dry run mode is enabled");
                "-avhPicn"
            } else {
                "-avhPi"
            };

            let folders = [
                ("pool/", "pool", false),
                ("dists/", "dists", true),
                ("pool/", "pool", true),
                ("index.html", "", false),
                ("kpcyrd.pgp", "", false),
            ];

            for (src, dst, delete) in folders {
                info!("Syncing {src:?}...");
                let mut cmd = process::Command::new("rsync");
                cmd.arg(flags);
                if delete {
                    cmd.arg("--delete");
                }
                cmd.arg("--");
                cmd.arg(src);
                cmd.arg(format!("{target}/{dst}"));
                status_to_err("rsync", cmd.status()?)?;
            }
        }
    }

    Ok(())
}
