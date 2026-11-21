use crate::config::OtelExporter;
use crate::metrics::Result;
use crate::metrics::validation::validate_tag_key;
use crate::metrics::validation::validate_tag_value;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum MetricsExporter {
    Otlp(OtelExporter),
}

#[derive(Clone, Debug)]
pub struct MetricsConfig {
    pub(crate) http_client_factory: codex_http_client::HttpClientFactory,
    pub(crate) exporter: MetricsExporter,
    pub(crate) runtime_reader: bool,
    pub(crate) default_tags: BTreeMap<String, String>,
}

impl MetricsConfig {
    pub fn with_http_client_factory(
        mut self,
        factory: codex_http_client::HttpClientFactory,
    ) -> Self {
        self.http_client_factory = factory;
        self
    }

    pub fn otlp(
        _environment: impl Into<String>,
        _service_name: impl Into<String>,
        _service_version: impl Into<String>,
        exporter: OtelExporter,
    ) -> Self {
        Self {
            http_client_factory: codex_http_client::HttpClientFactory::new(
                codex_http_client::OutboundProxyPolicy::ReqwestDefault,
            ),
            exporter: MetricsExporter::Otlp(exporter),
            runtime_reader: false,
            default_tags: BTreeMap::new(),
        }
    }

    /// Override the interval between periodic metric exports.
    pub fn with_export_interval(self, _interval: Duration) -> Self {
        self
    }

    /// Enable a manual reader for on-demand runtime snapshots.
    pub fn with_runtime_reader(mut self) -> Self {
        self.runtime_reader = true;
        self
    }

    /// Add a default tag that will be sent with every metric.
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Result<Self> {
        let key = key.into();
        let value = value.into();
        validate_tag_key(&key)?;
        validate_tag_value(&value)?;
        self.default_tags.insert(key, value);
        Ok(self)
    }
}
