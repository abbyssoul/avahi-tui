use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    net::IpAddr,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use crate::{
    cli::Cli,
    service::{ServiceId, ServiceRecord},
};

#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Upsert(ServiceRecord),
    Remove(ServiceId),
    Status(String),
}

pub struct DiscoveryHandle {
    receiver: Option<mpsc::Receiver<DiscoveryEvent>>,
    child: Option<Arc<Mutex<Child>>>,
}

impl DiscoveryHandle {
    pub fn take_receiver(&mut self) -> mpsc::Receiver<DiscoveryEvent> {
        self.receiver
            .take()
            .expect("discovery receiver can only be taken once")
    }
}

impl Drop for DiscoveryHandle {
    fn drop(&mut self) {
        if let Some(child) = &self.child
            && let Ok(mut child) = child.lock()
        {
            let _ = child.kill();
        }
    }
}

pub fn start(cli: &Cli) -> DiscoveryHandle {
    let (tx, rx) = mpsc::channel();

    if cli.fake_discovery {
        spawn_fake(cli.domain.clone(), cli.service_type.clone(), tx);
        return DiscoveryHandle {
            receiver: Some(rx),
            child: None,
        };
    }

    match spawn_avahi(cli, tx.clone()) {
        Ok(child) => DiscoveryHandle {
            receiver: Some(rx),
            child: Some(child),
        },
        Err(err) => {
            let _ = tx.send(DiscoveryEvent::Status(format!(
                "avahi-browse unavailable ({err}); using sample records"
            )));
            spawn_fake(cli.domain.clone(), cli.service_type.clone(), tx);
            DiscoveryHandle {
                receiver: Some(rx),
                child: None,
            }
        }
    }
}

fn spawn_avahi(cli: &Cli, tx: mpsc::Sender<DiscoveryEvent>) -> std::io::Result<Arc<Mutex<Child>>> {
    let mut command = Command::new("avahi-browse");
    command.arg("-p").arg("-r").arg("-d").arg(&cli.domain);
    if let Some(service_type) = &cli.service_type {
        command.arg(service_type);
    } else {
        command.arg("-a");
    }
    command.stdout(Stdio::piped()).stderr(Stdio::null());

    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let child = Arc::new(Mutex::new(child));

    thread::spawn(move || {
        let _ = tx.send(DiscoveryEvent::Status(
            "listening with avahi-browse".to_string(),
        ));
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(event) = parse_avahi_line(&line) {
                let _ = tx.send(event);
            }
        }
        let _ = tx.send(DiscoveryEvent::Status("avahi-browse stopped".to_string()));
    });

    Ok(child)
}

fn parse_avahi_line(line: &str) -> Option<DiscoveryEvent> {
    let parts = line.split(';').collect::<Vec<_>>();
    match parts.first().copied() {
        Some("=") if parts.len() >= 9 => {
            let mut record = ServiceRecord::new(parts[3], parts[4], parts[5]);
            record.hostname = non_empty(parts.get(6).copied());
            record.address = parts.get(7).and_then(|value| value.parse::<IpAddr>().ok());
            record.port = parts.get(8).and_then(|value| value.parse::<u16>().ok());
            if let Some(txt) = parts.get(9) {
                record.txt = parse_txt(txt);
            }
            Some(DiscoveryEvent::Upsert(record.with_instance_id()))
        }
        Some("+") if parts.len() >= 6 => {
            let record = ServiceRecord::new(parts[3], parts[4], parts[5]).with_instance_id();
            Some(DiscoveryEvent::Upsert(record))
        }
        Some("-") if parts.len() >= 6 => {
            let record = ServiceRecord::new(parts[3], parts[4], parts[5]).with_instance_id();
            Some(DiscoveryEvent::Remove(record.id))
        }
        _ => None,
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn parse_txt(value: &str) -> BTreeMap<String, String> {
    let mut txt = BTreeMap::new();
    for item in value.split_whitespace() {
        let item = item.trim_matches('"');
        if let Some((key, value)) = item.split_once('=') {
            txt.insert(key.to_string(), value.to_string());
        } else if !item.is_empty() {
            txt.insert(item.to_string(), String::new());
        }
    }
    txt
}

fn spawn_fake(
    domain: String,
    service_type_filter: Option<String>,
    tx: mpsc::Sender<DiscoveryEvent>,
) {
    thread::spawn(move || {
        let _ = tx.send(DiscoveryEvent::Status(
            "using sample discovery records".to_string(),
        ));
        let mut records = fake_records(&domain);
        if let Some(service_type) = service_type_filter {
            records.retain(|record| record.service_type == service_type);
        }
        for record in records {
            let _ = tx.send(DiscoveryEvent::Upsert(record));
            thread::sleep(Duration::from_millis(150));
        }
    });
}

fn fake_records(domain: &str) -> Vec<ServiceRecord> {
    let mut ssh_a = ServiceRecord::new("workstation", "_ssh._tcp", domain);
    ssh_a.hostname = Some("workstation.local".to_string());
    ssh_a.address = Some("192.168.1.20".parse().unwrap());
    ssh_a.port = Some(22);

    let mut ssh_b = ssh_a.clone();
    ssh_b.address = Some("192.168.1.21".parse().unwrap());

    let mut http = ServiceRecord::new("nas", "_http._tcp", domain);
    http.hostname = Some("nas.local".to_string());
    http.address = Some("192.168.1.30".parse().unwrap());
    http.port = Some(8080);
    http.txt.insert("path".to_string(), "/admin".to_string());

    let mut https = ServiceRecord::new("router", "_https._tcp", domain);
    https.hostname = Some("router.local".to_string());
    https.address = Some("192.168.1.1".parse().unwrap());
    https.port = Some(443);

    let unresolved = ServiceRecord::new("pending-printer", "_ipp._tcp", domain);

    vec![
        ssh_a.with_instance_id(),
        ssh_b.with_instance_id(),
        http.with_instance_id(),
        https.with_instance_id(),
        unresolved.with_instance_id(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_resolved_avahi_line() {
        let event = parse_avahi_line("=;eth0;IPv4;host;_ssh._tcp;local;host.local;192.168.1.2;22;")
            .unwrap();
        let DiscoveryEvent::Upsert(record) = event else {
            panic!("expected upsert");
        };
        assert_eq!(record.name, "host");
        assert_eq!(record.port, Some(22));
    }
}
