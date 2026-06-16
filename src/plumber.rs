use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Result, eyre};
use regex::Regex;

use crate::service::{ServiceGroup, ServiceRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionMode {
    Fork,
    Execute,
}

impl std::fmt::Display for ActionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionMode::Fork => f.write_str("fork"),
            ActionMode::Execute => f.write_str("execute"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandAction {
    pub description: Option<String>,
    pub command: String,
    pub mode: ActionMode,
}

#[derive(Debug, Clone)]
pub struct CommandConfig {
    pub name: String,
    pub description: Option<String>,
    pub requirements: Vec<String>,
    pub predicates: Vec<FieldPredicate>,
    pub action: CommandAction,
}

#[derive(Debug, Clone)]
pub struct FieldPredicate {
    pub field: String,
    pub predicate: Predicate,
}

#[derive(Debug, Clone)]
pub enum Predicate {
    Equals(String),
    Contains(String),
    Regex(Regex),
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub command: CommandConfig,
    pub matching_records: Vec<ServiceRecord>,
    pub needs_instance: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Matcher {
    commands: Vec<CommandConfig>,
}

impl Matcher {
    pub fn matches_group(&self, group: &ServiceGroup) -> Vec<MatchResult> {
        self.commands
            .iter()
            .filter_map(|command| {
                let matching_records: Vec<ServiceRecord> = group
                    .instances
                    .iter()
                    .filter(|record| command.matches_record(record))
                    .cloned()
                    .collect();
                if matching_records.is_empty() {
                    return None;
                }
                let needs_instance = command.needs_instance()
                    || matching_records.len() > 1 && command.has_instance_specific_template();
                Some(MatchResult {
                    command: command.clone(),
                    matching_records,
                    needs_instance,
                })
            })
            .collect()
    }

    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    pub fn commands(&self) -> &[CommandConfig] {
        &self.commands
    }
}

impl CommandConfig {
    fn matches_record(&self, record: &ServiceRecord) -> bool {
        self.predicates
            .iter()
            .all(|predicate| predicate.matches(record))
    }

    pub fn needs_instance(&self) -> bool {
        self.predicates
            .iter()
            .any(|predicate| is_instance_field(&predicate.field))
            || self.has_instance_specific_template()
    }

    fn has_instance_specific_template(&self) -> bool {
        self.action.command.contains("{address}") || self.action.command.contains("{port}")
    }
}

impl FieldPredicate {
    fn matches(&self, record: &ServiceRecord) -> bool {
        let Some(value) = record.field(&self.field) else {
            return false;
        };
        match &self.predicate {
            Predicate::Equals(expected) => value == *expected,
            Predicate::Contains(expected) => value.contains(expected),
            Predicate::Regex(regex) => regex.is_match(&value),
        }
    }
}

fn is_instance_field(field: &str) -> bool {
    matches!(field, "address" | "port")
}

#[derive(Debug, Default)]
pub struct MatcherBuilder {
    commands: Vec<CommandConfig>,
    /// command name -> (layer it was last defined in, index into `commands`)
    names: BTreeMap<String, (usize, usize)>,
    layer: usize,
}

impl MatcherBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a new override layer. Commands added afterwards override same-named
    /// commands from earlier layers; duplicates within one layer remain errors.
    pub fn start_layer(&mut self) {
        self.layer += 1;
    }

    pub fn add_str(&mut self, source_name: &str, source: &str) -> Result<()> {
        let command = parse_command_config(source_name, source)?;
        match self.names.get(&command.name).copied() {
            Some((layer, _)) if layer == self.layer => {
                return Err(eyre!(
                    "duplicate command name `{}` in {source_name}",
                    command.name
                ));
            }
            Some((_, index)) => {
                // Same name from an earlier layer: override it in place so the
                // command keeps its original position in the list.
                let name = command.name.clone();
                self.commands[index] = command;
                self.names.insert(name, (self.layer, index));
            }
            None => {
                let index = self.commands.len();
                self.names.insert(command.name.clone(), (self.layer, index));
                self.commands.push(command);
            }
        }
        Ok(())
    }

    pub fn add_file(&mut self, path: &Path) -> Result<()> {
        let source = fs::read_to_string(path)?;
        self.add_str(&path.display().to_string(), &source)
    }

    pub fn build(self) -> Matcher {
        Matcher {
            commands: self.commands,
        }
    }
}

/// System-wide command directory, loaded as the base layer for every run.
pub const SYSTEM_CONFIG_DIR: &str = "/etc/avahi-tui/commands";

/// Ordered list of command directories, lowest precedence first. Commands in a
/// later directory override same-named commands from an earlier one:
///
///   1. system-wide  (`/etc/avahi-tui/commands`)
///   2. user-local   (`$XDG_CONFIG_HOME/avahi-tui/commands` or `~/.config/...`)
///   3. command-line `--config-dir` entries, in the order given
pub fn config_dirs(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from(SYSTEM_CONFIG_DIR)];
    if let Some(home) = env::var_os("XDG_CONFIG_HOME") {
        dirs.push(PathBuf::from(home).join("avahi-tui").join("commands"));
    } else if let Some(home) = env::var_os("HOME") {
        dirs.push(
            PathBuf::from(home)
                .join(".config")
                .join("avahi-tui")
                .join("commands"),
        );
    }
    dirs.extend(extra.iter().cloned());
    dirs
}

pub fn load_from_dirs(builder: &mut MatcherBuilder, dirs: &[PathBuf]) -> Result<()> {
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        builder.start_layer();
        let mut files = fs::read_dir(dir)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect::<Vec<_>>();
        files.sort();
        for path in files {
            builder.add_file(&path)?;
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
struct RawConfig {
    metadata: BTreeMap<String, Value>,
    action: BTreeMap<String, Value>,
    predicates: BTreeMap<String, BTreeMap<String, Value>>,
}

#[derive(Debug, Clone)]
enum Section {
    Metadata,
    Action,
    Match(String),
}

#[derive(Debug, Clone)]
enum Value {
    String(String),
    Array(Vec<String>),
}

fn parse_command_config(source_name: &str, source: &str) -> Result<CommandConfig> {
    let raw = parse_minimal_toml(source_name, source)?;
    let name = required_string(&raw.metadata, "name", source_name)?;
    let description = optional_string(&raw.metadata, "description")?;
    let requirements = optional_array(&raw.metadata, "requirements")?;
    let action_description = optional_string(&raw.action, "description")?;
    let command = required_string(&raw.action, "command", source_name)?;
    let mode = match required_string(&raw.action, "mode", source_name)?.as_str() {
        "fork" => ActionMode::Fork,
        "execute" | "exec" => ActionMode::Execute,
        value => return Err(eyre!("{source_name}: invalid action mode `{value}`")),
    };

    let mut predicates = Vec::new();
    for (field, values) in raw.predicates {
        for (kind, value) in values {
            let value = match value {
                Value::String(value) => value,
                Value::Array(_) => {
                    return Err(eyre!(
                        "{source_name}: match `{field}.{kind}` must be a string"
                    ));
                }
            };
            let predicate = match kind.as_str() {
                "equals" => Predicate::Equals(value),
                "contains" => Predicate::Contains(value),
                "regex" => Predicate::Regex(Regex::new(&value)?),
                _ => {
                    return Err(eyre!(
                        "{source_name}: unsupported predicate `{field}.{kind}`"
                    ));
                }
            };
            predicates.push(FieldPredicate {
                field: field.clone(),
                predicate,
            });
        }
    }

    if predicates.is_empty() {
        return Err(eyre!(
            "{source_name}: command `{name}` has no match predicates"
        ));
    }

    Ok(CommandConfig {
        name,
        description,
        requirements,
        predicates,
        action: CommandAction {
            description: action_description,
            command,
            mode,
        },
    })
}

fn parse_minimal_toml(source_name: &str, source: &str) -> Result<RawConfig> {
    let mut raw = RawConfig::default();
    let mut section: Option<Section> = None;

    for (index, line) in source.lines().enumerate() {
        let line_no = index + 1;
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = &line[1..line.len() - 1];
            section = Some(match name {
                "metadata" => Section::Metadata,
                "action" => Section::Action,
                value if value.starts_with("match.") => {
                    Section::Match(value.trim_start_matches("match.").to_string())
                }
                _ => {
                    return Err(eyre!(
                        "{source_name}:{line_no}: unsupported section `{name}`"
                    ));
                }
            });
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(eyre!("{source_name}:{line_no}: expected key = value"));
        };
        let key = key.trim().to_string();
        let value = parse_value(source_name, line_no, value.trim())?;
        match &section {
            Some(Section::Metadata) => {
                raw.metadata.insert(key, value);
            }
            Some(Section::Action) => {
                raw.action.insert(key, value);
            }
            Some(Section::Match(field)) => {
                raw.predicates
                    .entry(field.clone())
                    .or_default()
                    .insert(key, value);
            }
            None => return Err(eyre!("{source_name}:{line_no}: key outside a section")),
        }
    }

    Ok(raw)
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '\\' if in_string => escaped = !escaped,
            '"' if !escaped => in_string = !in_string,
            '#' if !in_string => return &line[..index],
            _ => escaped = false,
        }
    }
    line
}

fn parse_value(source_name: &str, line_no: usize, value: &str) -> Result<Value> {
    if value.starts_with('"') {
        return Ok(Value::String(parse_string(source_name, line_no, value)?));
    }
    if value.starts_with('[') && value.ends_with(']') {
        let inner = value[1..value.len() - 1].trim();
        if inner.is_empty() {
            return Ok(Value::Array(Vec::new()));
        }
        let mut values = Vec::new();
        for item in split_array(inner) {
            values.push(parse_string(source_name, line_no, item.trim())?);
        }
        return Ok(Value::Array(values));
    }
    Err(eyre!(
        "{source_name}:{line_no}: only quoted strings and string arrays are supported"
    ))
}

fn parse_string(source_name: &str, line_no: usize, value: &str) -> Result<String> {
    if !value.starts_with('"') || !value.ends_with('"') || value.len() < 2 {
        return Err(eyre!("{source_name}:{line_no}: expected quoted string"));
    }
    let raw = &value[1..value.len() - 1];
    let mut parsed = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let Some(next) = chars.next() else {
                return Err(eyre!("{source_name}:{line_no}: trailing string escape"));
            };
            match next {
                'n' => parsed.push('\n'),
                't' => parsed.push('\t'),
                'r' => parsed.push('\r'),
                '"' => parsed.push('"'),
                '\\' => parsed.push('\\'),
                other => {
                    parsed.push('\\');
                    parsed.push(other);
                }
            }
        } else {
            parsed.push(ch);
        }
    }
    Ok(parsed)
}

fn split_array(value: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        match ch {
            '\\' if in_string => escaped = !escaped,
            '"' if !escaped => in_string = !in_string,
            ',' if !in_string => {
                result.push(&value[start..index]);
                start = index + 1;
            }
            _ => escaped = false,
        }
    }
    result.push(&value[start..]);
    result
}

fn required_string(
    values: &BTreeMap<String, Value>,
    key: &str,
    source_name: &str,
) -> Result<String> {
    optional_string(values, key)?.ok_or_else(|| eyre!("{source_name}: missing `{key}`"))
}

fn optional_string(values: &BTreeMap<String, Value>, key: &str) -> Result<Option<String>> {
    match values.get(key) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Array(_)) => Err(eyre!("`{key}` must be a string")),
        None => Ok(None),
    }
}

fn optional_array(values: &BTreeMap<String, Value>, key: &str) -> Result<Vec<String>> {
    match values.get(key) {
        Some(Value::Array(value)) => Ok(value.clone()),
        Some(Value::String(_)) => Err(eyre!("`{key}` must be an array")),
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_toml(name: &str, command: &str) -> String {
        format!(
            r#"
[metadata]
name = "{name}"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "{command}"
mode = "execute"
"#
        )
    }

    #[test]
    fn later_layers_override_earlier_commands() {
        let mut builder = MatcherBuilder::new();
        builder.start_layer(); // system
        builder
            .add_str("system/ssh", &command_toml("ssh", "ssh system"))
            .unwrap();
        builder
            .add_str("system/mosh", &command_toml("mosh", "mosh system"))
            .unwrap();
        builder.start_layer(); // user overlay
        builder
            .add_str("user/ssh", &command_toml("ssh", "ssh user"))
            .unwrap();

        let matcher = builder.build();
        assert_eq!(matcher.command_count(), 2);
        // The override keeps the command in its original position.
        assert_eq!(matcher.commands()[0].name, "ssh");
        assert_eq!(matcher.commands()[0].action.command, "ssh user");
        assert_eq!(matcher.commands()[1].name, "mosh");
    }

    #[test]
    fn duplicate_within_one_layer_is_rejected() {
        let mut builder = MatcherBuilder::new();
        builder.start_layer();
        builder
            .add_str("a", &command_toml("ssh", "ssh a"))
            .unwrap();
        let err = builder
            .add_str("b", &command_toml("ssh", "ssh b"))
            .unwrap_err();
        assert!(err.to_string().contains("duplicate command name"));
    }

    #[test]
    fn config_dirs_layer_system_then_user_then_extras() {
        let extra = PathBuf::from("/tmp/avahi-extra");
        let dirs = config_dirs(std::slice::from_ref(&extra));
        assert_eq!(dirs.first(), Some(&PathBuf::from(SYSTEM_CONFIG_DIR)));
        assert_eq!(dirs.last(), Some(&extra));
    }

    #[test]
    fn parses_structured_matcher() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "test",
                r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh {hostname}"
mode = "execute"
"#,
            )
            .unwrap();

        let matcher = builder.build();
        assert_eq!(matcher.command_count(), 1);

        let command = &matcher.commands()[0];
        assert_eq!(command.name, "ssh");
        assert_eq!(command.description, None);
        assert!(command.requirements.is_empty());
        assert_eq!(command.predicates.len(), 1);
        let predicate = &command.predicates[0];
        assert_eq!(predicate.field, "service_type");
        match &predicate.predicate {
            Predicate::Equals(value) => assert_eq!(value, "_ssh._tcp"),
            _ => panic!("unexpected predicate type"),
        }
        assert_eq!(command.action.description, None);
        assert_eq!(command.action.command, "ssh {hostname}");
        assert_eq!(command.action.mode, ActionMode::Execute);
    }

    #[test]
    fn matcher_filters_records() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "ssh",
                r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh {hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        let matcher = builder.build();
        let mut record = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        record.hostname = Some("alpha.local".to_string());
        let group =
            crate::service::group_records(&[record], crate::service::GroupingMode::LogicalService)
                .remove(0);
        assert_eq!(matcher.matches_group(&group).len(), 1);
    }

    #[test]
    fn lists_loaded_command_names() {
        let mut builder = MatcherBuilder::new();
        builder
            .add_str(
                "ssh",
                r#"
[metadata]
name = "ssh"

[match.service_type]
equals = "_ssh._tcp"

[action]
command = "ssh {hostname}"
mode = "execute"
"#,
            )
            .unwrap();
        builder
            .add_str(
                "open-http",
                r#"
[metadata]
name = "open-http"

[match.service_type]
equals = "_http._tcp"

[action]
command = "xdg-open http://{hostname}:{port}"
mode = "execute"
"#,
            )
            .unwrap();

        let matcher = builder.build();
        let names = matcher
            .commands()
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["ssh", "open-http"]);
    }
}
