use crate::config::OtelExporter;
use crate::metrics::MetricsError;
use crate::metrics::Result;
use crate::metrics::config::MetricsConfig;
use crate::metrics::config::MetricsExporter;
use crate::metrics::timer::Timer;
use crate::metrics::validation::validate_metric_name;
use crate::metrics::validation::validate_tag_key;
use crate::metrics::validation::validate_tag_value;
use crate::metrics::validation::validate_tags;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

/// Runtime metrics snapshot. Metrics are never aggregated, so it is always empty.
#[derive(Clone, Debug, Default)]
pub struct MetricsSnapshot;

#[derive(Debug)]
pub(super) struct MetricsClientInner {
    pub(super) network_policy: codex_http_client::NetworkPolicy,
    runtime_reader: bool,
    requires_analytics_consent: bool,
}

impl MetricsClientInner {
    /// Validates the measurement like a real exporter would and then discards it.
    fn record(&self, name: &str, tags: &[(&str, &str)]) -> Result<()> {
        validate_metric_name(name)?;
        for (key, value) in tags {
            validate_tag_key(key)?;
            validate_tag_value(value)?;
        }
        Ok(())
    }

    fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]) -> Result<()> {
        validate_metric_name(name)?;
        if inc < 0 {
            return Err(MetricsError::NegativeCounterIncrement {
                name: name.to_string(),
                inc,
            });
        }
        self.record(name, tags)
    }
}

/// Metrics client used by Codex. Measurements are validated and discarded.
#[derive(Clone, Debug)]
pub struct MetricsClient {
    // Keep the original provider so its owner only shuts down its own exporter.
    pub(super) inner: Arc<MetricsClientInner>,
    // Installed clients share this slot, so existing clones follow account changes.
    pub(super) active: Option<Arc<RwLock<Arc<MetricsClientInner>>>>,
}

impl MetricsClient {
    /// Build a metrics client from configuration and validate defaults.
    pub fn new(config: MetricsConfig) -> Result<Self> {
        let MetricsConfig {
            http_client_factory,
            exporter,
            runtime_reader,
            default_tags,
        } = config;

        validate_tags(&default_tags)?;

        let requires_analytics_consent =
            matches!(exporter, MetricsExporter::Otlp(OtelExporter::Statsig));

        Ok(Self {
            inner: Arc::new(MetricsClientInner {
                network_policy: http_client_factory.network_policy().clone(),
                runtime_reader,
                requires_analytics_consent,
            }),
            active: None,
        })
    }

    pub(super) fn active_inner(&self) -> Arc<MetricsClientInner> {
        match &self.active {
            Some(active) => active
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            None => Arc::clone(&self.inner),
        }
    }

    /// Validate a single counter increment.
    pub fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]) -> Result<()> {
        self.active_inner().counter(name, inc, tags)
    }

    /// Validate a counter while applying analytics consent only to the built-in exporter.
    pub fn counter_with_analytics_consent(
        &self,
        name: &str,
        inc: i64,
        tags: &[(&str, &str)],
        analytics_enabled: bool,
    ) -> Result<()> {
        let inner = self.active_inner();
        if inner.requires_analytics_consent && !analytics_enabled {
            return Ok(());
        }
        inner.counter(name, inc, tags)
    }

    /// Validate a single counter increment with an instrument description.
    pub fn counter_with_description(
        &self,
        name: &str,
        _description: &str,
        inc: i64,
        tags: &[(&str, &str)],
    ) -> Result<()> {
        self.active_inner().counter(name, inc, tags)
    }

    /// Validate a single histogram sample.
    pub fn histogram(&self, name: &str, _value: i64, tags: &[(&str, &str)]) -> Result<()> {
        self.active_inner().record(name, tags)
    }

    /// Validate a single histogram sample using explicit bucket boundaries.
    pub fn histogram_with_boundaries(
        &self,
        name: &str,
        _value: i64,
        _boundaries: &[f64],
        tags: &[(&str, &str)],
    ) -> Result<()> {
        self.active_inner().record(name, tags)
    }

    /// Validate a single gauge measurement.
    pub fn gauge(&self, name: &str, _value: i64, tags: &[(&str, &str)]) -> Result<()> {
        self.active_inner().record(name, tags)
    }

    /// Validate a single gauge measurement with an instrument description.
    pub fn gauge_with_description(
        &self,
        name: &str,
        _description: &str,
        _value: i64,
        tags: &[(&str, &str)],
    ) -> Result<()> {
        self.active_inner().record(name, tags)
    }

    /// Validate a gauge callback registration. The callback is never invoked.
    pub fn register_observable_gauge_with_description(
        &self,
        name: &str,
        _description: &str,
        _observe: impl Fn() -> i64 + Send + Sync + 'static,
        tags: &[(&str, &str)],
    ) -> Result<()> {
        self.active_inner().record(name, tags)
    }

    /// Validate a duration in milliseconds.
    pub fn record_duration(
        &self,
        name: &str,
        _duration: Duration,
        tags: &[(&str, &str)],
    ) -> Result<()> {
        self.active_inner().record(name, tags)
    }

    /// Validate a duration supplied as fractional milliseconds.
    pub(crate) fn record_duration_ms_f64(
        &self,
        name: &str,
        _duration_ms: f64,
        tags: &[(&str, &str)],
    ) -> Result<()> {
        self.active_inner().record(name, tags)
    }

    /// Validate a duration in seconds with an instrument description.
    pub fn record_duration_seconds_with_description(
        &self,
        name: &str,
        _description: &str,
        _duration: Duration,
        tags: &[(&str, &str)],
    ) -> Result<()> {
        self.active_inner().record(name, tags)
    }

    pub fn start_timer(
        &self,
        name: &str,
        tags: &[(&str, &str)],
    ) -> std::result::Result<Timer, MetricsError> {
        Ok(Timer::new(name, tags, self))
    }

    /// Return an empty runtime metrics snapshot.
    pub fn snapshot(&self) -> Result<MetricsSnapshot> {
        if !self.active_inner().runtime_reader {
            return Err(MetricsError::RuntimeSnapshotUnavailable);
        }
        Ok(MetricsSnapshot)
    }

    /// Stop recording. There is nothing to flush.
    pub fn shutdown(&self) -> Result<()> {
        super::buffered::GLOBAL.suspend(self);
        Ok(())
    }
}
