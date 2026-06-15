use std::{collections::BTreeMap, net::IpAddr, sync::mpsc, thread, time::Duration};

use dns_sd_native::{
    BrowseEvent, DiscoveredService, RemovedService, ServiceBrowserBuilder, TxtRecord,
};
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;

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
    shutdown: CancellationToken,
    worker: Option<thread::JoinHandle<()>>,
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
        self.shutdown.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn start(cli: &Cli) -> DiscoveryHandle {
    let (tx, rx) = mpsc::channel();
    let shutdown = CancellationToken::new();

    if cli.fake_discovery {
        spawn_fake(cli.domain.clone(), cli.service_type.clone(), tx);
        return DiscoveryHandle {
            receiver: Some(rx),
            shutdown,
            worker: None,
        };
    }

    let worker = spawn_browser(cli, tx, shutdown.clone());
    DiscoveryHandle {
        receiver: Some(rx),
        shutdown,
        worker: Some(worker),
    }
}

fn spawn_browser(
    cli: &Cli,
    tx: mpsc::Sender<DiscoveryEvent>,
    shutdown: CancellationToken,
) -> thread::JoinHandle<()> {
    let domain = cli.domain.clone();
    let service_type_filter = cli.service_type.clone();

    thread::spawn(move || {
        let runtime = match Builder::new_multi_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = tx.send(DiscoveryEvent::Status(format!(
                    "failed to start mDNS runtime ({err}); using sample records"
                )));
                spawn_fake(domain, service_type_filter, tx);
                return;
            }
        };

        runtime.block_on(browse_loop(domain, service_type_filter, tx, shutdown));
    })
}

/// Browses DNS-SD services via `dns-sd-native`. With no `--service-type` the
/// browser discovers **every** service type on the link through the DNS-SD
/// service-type meta-query, so there is no curated list of types to maintain.
async fn browse_loop(
    domain: String,
    service_type_filter: Option<String>,
    tx: mpsc::Sender<DiscoveryEvent>,
    shutdown: CancellationToken,
) {
    let mut builder = ServiceBrowserBuilder::new();
    if let Some(service_type) = &service_type_filter {
        builder.service_type(service_type);
    }
    if !domain.is_empty() {
        builder.domain(&domain);
    }

    let mut browser = match builder.browse().await {
        Ok(browser) => browser,
        Err(err) => {
            let _ = tx.send(DiscoveryEvent::Status(format!(
                "mDNS discovery unavailable ({err}); using sample records"
            )));
            spawn_fake(domain, service_type_filter, tx);
            return;
        }
    };

    let _ = tx.send(DiscoveryEvent::Status(match &service_type_filter {
        Some(service_type) => format!("browsing {service_type} over mDNS"),
        None => "browsing all service types over mDNS".to_string(),
    }));

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            event = browser.recv() => match event {
                Some(Ok(BrowseEvent::Found(service))) => {
                    for record in records_from_discovery(&service) {
                        if tx.send(DiscoveryEvent::Upsert(record)).is_err() {
                            return;
                        }
                    }
                }
                Some(Ok(BrowseEvent::Removed(service))) => {
                    if tx.send(DiscoveryEvent::Remove(id_from_removal(&service))).is_err() {
                        return;
                    }
                }
                Some(Err(err)) => {
                    let _ = tx.send(DiscoveryEvent::Status(format!("mDNS browse error: {err}")));
                }
                None => break,
            }
        }
    }
    // Dropping `browser` here stops the underlying native browse operation.
}

/// Expands a resolved [`DiscoveredService`] into one [`ServiceRecord`] per
/// resolved address (services can advertise several). A service that resolves
/// without any addresses still yields a single record keyed by host and port.
fn records_from_discovery(service: &DiscoveredService) -> Vec<ServiceRecord> {
    let txt = txt_map(&service.txt_records);
    let hostname = Some(service.host_name.as_str());
    let port = Some(service.port);

    if service.addresses.is_empty() {
        return vec![upsert_record(
            &service.name,
            &service.service_type,
            &service.domain,
            hostname,
            None,
            port,
            txt,
        )];
    }

    service
        .addresses
        .iter()
        .map(|address| {
            upsert_record(
                &service.name,
                &service.service_type,
                &service.domain,
                hostname,
                Some(&address.to_string()),
                port,
                txt.clone(),
            )
        })
        .collect()
}

fn id_from_removal(service: &RemovedService) -> ServiceId {
    ServiceRecord::new(&service.name, &service.service_type, &service.domain)
        .with_instance_id()
        .id
}

/// Flattens DNS-SD TXT records into the `key -> value` map carried by a
/// [`ServiceRecord`]. Key-only entries (advertised without an `=`) and entries
/// with an empty value both map to an empty string.
fn txt_map(records: &[TxtRecord]) -> BTreeMap<String, String> {
    records
        .iter()
        .map(|record| {
            let value = record
                .value
                .as_deref()
                .map(|value| String::from_utf8_lossy(value).into_owned())
                .unwrap_or_default();
            (record.key.clone(), value)
        })
        .collect()
}

/// Builds a resolved [`ServiceRecord`] from the individual fields reported by a
/// browse event. Kept separate from the `dns-sd-native` types so it can be unit
/// tested without standing up the mDNS stack.
fn upsert_record(
    name: &str,
    service_type: &str,
    domain: &str,
    hostname: Option<&str>,
    address: Option<&str>,
    port: Option<u16>,
    txt: BTreeMap<String, String>,
) -> ServiceRecord {
    let mut record = ServiceRecord::new(name, service_type, domain);
    record.hostname = hostname
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    record.address = address.and_then(|value| value.parse::<IpAddr>().ok());
    record.port = port.filter(|port| *port != 0);
    record.txt = txt;
    record.with_instance_id()
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

    fn discovered(name: &str, addresses: &[&str]) -> DiscoveredService {
        DiscoveredService {
            name: name.to_string(),
            service_type: "_ssh._tcp".to_string(),
            domain: "local".to_string(),
            host_name: "workstation.local".to_string(),
            port: 22,
            addresses: addresses.iter().map(|a| a.parse().unwrap()).collect(),
            txt_records: vec![TxtRecord {
                key: "path".to_string(),
                value: Some(b"/admin".to_vec()),
            }],
            interface_index: None,
        }
    }

    #[test]
    fn builds_resolved_record_from_browse_fields() {
        let mut txt = BTreeMap::new();
        txt.insert("path".to_string(), "/admin".to_string());

        let record = upsert_record(
            "nas",
            "_http._tcp",
            "local",
            Some("nas.local"),
            Some("192.168.1.30"),
            Some(8080),
            txt,
        );

        assert_eq!(record.name, "nas");
        assert_eq!(record.hostname.as_deref(), Some("nas.local"));
        assert_eq!(record.address, Some("192.168.1.30".parse().unwrap()));
        assert_eq!(record.port, Some(8080));
        assert_eq!(record.txt.get("path").map(String::as_str), Some("/admin"));
        assert!(record.has_instance_data());
    }

    #[test]
    fn blank_host_and_zero_port_become_unresolved() {
        let record = upsert_record(
            "pending",
            "_ipp._tcp",
            "local",
            Some(""),
            Some(""),
            Some(0),
            BTreeMap::new(),
        );

        assert_eq!(record.hostname, None);
        assert_eq!(record.address, None);
        assert_eq!(record.port, None);
        assert!(!record.has_instance_data());
    }

    #[test]
    fn multiple_addresses_become_distinct_records() {
        let service = discovered("workstation", &["192.168.1.20", "192.168.1.21"]);
        let records = records_from_discovery(&service);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].name, "workstation");
        assert_eq!(records[0].port, Some(22));
        assert_eq!(records[0].txt.get("path").map(String::as_str), Some("/admin"));
        let addresses = records
            .iter()
            .filter_map(|record| record.address)
            .collect::<Vec<_>>();
        assert!(addresses.contains(&"192.168.1.20".parse().unwrap()));
        assert!(addresses.contains(&"192.168.1.21".parse().unwrap()));
    }

    #[test]
    fn addressless_service_still_yields_one_record() {
        let service = discovered("workstation", &[]);
        let records = records_from_discovery(&service);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].hostname.as_deref(), Some("workstation.local"));
        assert_eq!(records[0].address, None);
        assert!(records[0].has_instance_data());
    }

    #[test]
    fn removal_id_matches_pending_registration_key() {
        let removed = RemovedService {
            name: "workstation".to_string(),
            service_type: "_ssh._tcp".to_string(),
            domain: "local".to_string(),
            interface_index: None,
        };

        let id = id_from_removal(&removed);
        assert_eq!(id.registration_key(), "workstation|_ssh._tcp|local");
    }

    #[test]
    fn key_only_txt_entry_maps_to_empty_value() {
        let records = vec![TxtRecord {
            key: "flag".to_string(),
            value: None,
        }];
        let map = txt_map(&records);
        assert_eq!(map.get("flag").map(String::as_str), Some(""));
    }
}
