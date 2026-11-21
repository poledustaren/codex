use crate::config::OtelExporter;
use crate::config::OtelSettings;
use crate::config::StatsigMetricsSettings;
use crate::metrics::MetricsClient;
use crate::metrics::MetricsConfig;
use crate::targets::is_log_export_target;
use crate::targets::is_trace_safe_target;
use std::error::Error;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tracing::debug;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Identity;
use tracing_subscriber::registry::LookupSpan;

/// Telemetry provider. Logs and traces are never exported, and metrics are discarded.
pub struct OtelProvider {
    pub metrics: Option<MetricsClient>,
    shutdown_started: AtomicBool,
}

impl OtelProvider {
    /// Stops recording metrics at most once.
    pub fn shutdown(&self) {
        if self.shutdown_started.swap(/*val*/ true, Ordering::AcqRel) {
            return;
        }

        if let Some(metrics) = &self.metrics {
            let _ = metrics.shutdown();
        }
    }

    /// Shuts the provider down. There are no exporters, so this completes immediately.
    pub async fn shutdown_with_timeout(self, _timeout: Duration) -> io::Result<()> {
        self.shutdown();
        Ok(())
    }

    pub fn try_new(settings: &OtelSettings) -> Result<Option<Self>, Box<dyn Error>> {
        let log_enabled = !matches!(settings.exporter, OtelExporter::None);
        let trace_enabled = !matches!(settings.trace_exporter, OtelExporter::None);
        let metric_exporter = crate::config::resolve_exporter(&settings.metrics_exporter);
        let metrics_enabled = !matches!(metric_exporter, OtelExporter::None);

        if !log_enabled && !trace_enabled && !metrics_enabled {
            crate::metrics::buffered::GLOBAL.disable();
            debug!("No OTEL exporter enabled in settings.");
            return Ok(None);
        }

        // Keep span attribute checks aligned with config loading when traces are exported.
        if trace_enabled {
            crate::config::validate_span_attributes(&settings.span_attributes)?;
        }
        crate::trace_context::validate_tracestate_entries(&settings.tracestate)?;

        let metrics = if metrics_enabled {
            let mut config = MetricsConfig::otlp(
                settings.environment.clone(),
                settings.service_name.clone(),
                settings.service_version.clone(),
                settings.metrics_exporter.clone(),
            )
            .with_http_client_factory(settings.http_client_factory.clone());
            if settings.runtime_metrics {
                config = config.with_runtime_reader();
            }
            Some(MetricsClient::new(config)?)
        } else {
            None
        };

        let mut provider = Self {
            metrics,
            shutdown_started: AtomicBool::default(),
        };

        if let Some(metrics) = provider.metrics.as_mut() {
            *metrics = crate::metrics::install_global(metrics.clone());
            if matches!(settings.metrics_exporter, OtelExporter::Statsig) {
                crate::metrics::install_global_statsig_settings(StatsigMetricsSettings {
                    environment: settings.environment.clone(),
                });
            }
        } else {
            crate::metrics::buffered::GLOBAL.disable();
        }
        Ok(Some(provider))
    }

    /// Log export is not available, so there is never a layer to install.
    pub fn logger_layer<S>(&self) -> Option<impl Layer<S> + Send + Sync>
    where
        S: tracing::Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
    {
        self.logger_export_layer::<S>()
    }

    /// Log export is not available, so there is never a layer to install.
    pub fn logger_export_layer<S>(&self) -> Option<impl Layer<S> + Send + Sync>
    where
        S: tracing::Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
    {
        None::<Identity>
    }

    /// Trace export is not available, so there is never a layer to install.
    pub fn tracing_layer<S>(&self) -> Option<impl Layer<S> + Send + Sync>
    where
        S: tracing::Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
    {
        None::<Identity>
    }

    /// Returns a permanent trace layer that does nothing.
    pub fn reloadable_tracing_layer<S>(_service_name: &'static str) -> impl Layer<S> + Send + Sync
    where
        S: tracing::Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
    {
        Identity::new()
    }

    pub fn codex_export_filter(meta: &tracing::Metadata<'_>) -> bool {
        Self::log_export_filter(meta)
    }

    pub fn log_export_filter(meta: &tracing::Metadata<'_>) -> bool {
        is_log_export_target(meta.target())
    }

    pub fn trace_export_filter(meta: &tracing::Metadata<'_>) -> bool {
        let target = meta.target();
        if meta.is_span() {
            !matches!(target.split("::").next(), Some("h2" | "tokio" | "runtime"))
        } else {
            is_trace_safe_target(target)
        }
    }

    pub fn metrics(&self) -> Option<&MetricsClient> {
        self.metrics.as_ref()
    }
}

impl Drop for OtelProvider {
    fn drop(&mut self) {
        self.shutdown();
    }
}
