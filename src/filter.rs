use std::collections::BTreeSet;

use crate::service::{GroupingMode, ServiceRecord};

#[derive(Debug, Clone)]
pub struct FilterState {
    pub text_query: String,
    pub enabled_service_types: BTreeSet<String>,
    pub disabled_service_types: BTreeSet<String>,
    pub grouping: GroupingMode,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            text_query: String::new(),
            enabled_service_types: BTreeSet::new(),
            disabled_service_types: BTreeSet::new(),
            grouping: GroupingMode::LogicalService,
        }
    }
}

impl FilterState {
    pub fn sync_service_types(&mut self, records: &[ServiceRecord]) {
        for record in records {
            if !self.disabled_service_types.contains(&record.service_type) {
                self.enabled_service_types
                    .insert(record.service_type.clone());
            }
        }
    }

    pub fn discovered_types(records: &[ServiceRecord]) -> Vec<String> {
        records
            .iter()
            .map(|record| record.service_type.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn toggle_service_type(&mut self, service_type: &str) {
        if self.enabled_service_types.remove(service_type) {
            self.disabled_service_types.insert(service_type.to_string());
        } else {
            self.enabled_service_types.insert(service_type.to_string());
            self.disabled_service_types.remove(service_type);
        }
    }

    pub fn clear_text(&mut self) {
        self.text_query.clear();
    }

    pub fn apply(&self, records: &[ServiceRecord]) -> Vec<ServiceRecord> {
        records
            .iter()
            .filter(|record| self.enabled_service_types.contains(&record.service_type))
            .filter(|record| {
                self.text_query.trim().is_empty()
                    || fuzzy_match(&record.searchable_text(), self.text_query.trim())
            })
            .cloned()
            .collect()
    }
}

pub fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    let mut chars = haystack.chars().flat_map(char::to_lowercase);
    for needle_char in needle.chars().flat_map(char::to_lowercase) {
        if !chars.any(|haystack_char| haystack_char == needle_char) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_match_accepts_subsequence() {
        assert!(fuzzy_match("Kitchen Printer _ipp._tcp", "kpr"));
        assert!(!fuzzy_match("Kitchen Printer", "zx"));
    }

    #[test]
    fn type_filter_and_text_filter_combine() {
        let ssh = ServiceRecord::new("alpha", "_ssh._tcp", "local");
        let http = ServiceRecord::new("beta", "_http._tcp", "local");
        let mut filter = FilterState::default();
        filter.sync_service_types(&[ssh.clone(), http.clone()]);
        filter.toggle_service_type("_http._tcp");
        filter.text_query = "alp".to_string();

        let visible = filter.apply(&[ssh, http]);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "alpha");
    }
}
