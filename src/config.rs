use color_eyre::eyre::Result;

use crate::{
    cli::{Cli, CliCommand},
    keymap::{self, KeyBindings},
    plumber::{self, Matcher, MatcherBuilder},
};

pub fn load_matcher(cli: &Cli) -> Result<Matcher> {
    let mut builder = MatcherBuilder::new();

    let dirs = if cli.command == CliCommand::ListCommands && !cli.config_dirs.is_empty() {
        cli.config_dirs.clone()
    } else {
        plumber::config_dirs(&cli.config_dirs)
    };
    plumber::load_from_dirs(&mut builder, &dirs)?;
    Ok(builder.build())
}

pub fn load_keybindings() -> Result<KeyBindings> {
    KeyBindings::load(&keymap::default_config_paths())
}
