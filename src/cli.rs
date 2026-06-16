use std::path::PathBuf;

use clap::{Arg, ArgAction, Command, error::ErrorKind};
use color_eyre::eyre::Result;

#[derive(Debug, Clone)]
pub struct Cli {
    pub domain: String,
    pub config_dirs: Vec<PathBuf>,
    pub service_type: Option<String>,
    pub fake_discovery: bool,
    pub command: CliCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    Run,
    ListCommands,
}

pub fn parse() -> Result<Cli> {
    let matches = Command::new("avahi-tui")
        .about("TUI browser and launcher for DNS-SD services")
        .arg(
            // A flag rather than a positional so it never competes with the
            // subcommand slot — `avahi-tui <unknown>` now errors instead of
            // being silently treated as a domain name.
            Arg::new("domain")
                .long("domain")
                .short('d')
                .help("DNS-SD domain to browse")
                .default_value("local")
                .value_name("DOMAIN"),
        )
        .arg(
            Arg::new("config-dir")
                .long("config-dir")
                .help("Extra command directory; repeatable, each overlays the previous")
                .action(ArgAction::Append)
                .value_name("PATH"),
        )
        .arg(
            Arg::new("service-type")
                .long("service-type")
                .help("Limit discovery to one DNS-SD service type")
                .value_name("TYPE"),
        )
        .arg(
            Arg::new("fake-discovery")
                .long("fake-discovery")
                .help("Use built-in sample records instead of mDNS discovery")
                .action(ArgAction::SetTrue),
        )
        .subcommand(
            Command::new("list-commands")
                .about("Validate and list registered command configs")
                .arg(
                    Arg::new("config-dir")
                        .long("config-dir")
                        .help("Command config directory to load")
                        .action(ArgAction::Append)
                        .value_name("PATH"),
                ),
        )
        .get_matches();

    // `--config-dir` is repeatable and is read from whichever argument context
    // applies: the `list-commands` subcommand carries its own copy of the flag,
    // while a plain run reads the top-level one.
    let (command, config_dirs) = match matches.subcommand() {
        None => (CliCommand::Run, collect_config_dirs(&matches)),
        Some(("list-commands", sub)) => (CliCommand::ListCommands, collect_config_dirs(sub)),
        Some((name, _)) => {
            return Err(clap::Error::raw(
                ErrorKind::InvalidSubcommand,
                format!(
                    "unknown subcommand `{name}`; use `list-commands` to see available commands\n"
                ),
            )
            .into());
        }
    };

    Ok(Cli {
        domain: matches
            .get_one::<String>("domain")
            .cloned()
            .unwrap_or_else(|| "local".to_string()),
        config_dirs,
        service_type: matches.get_one::<String>("service-type").cloned(),
        fake_discovery: matches.get_flag("fake-discovery"),
        command,
    })
}

fn collect_config_dirs(matches: &clap::ArgMatches) -> Vec<PathBuf> {
    matches
        .get_many::<String>("config-dir")
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .collect()
}
