use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

// Installs the process-global Prometheus recorder. Recording elsewhere
// (middleware, handlers) is just `metrics::counter!`/`histogram!` macro
// calls against that global recorder — no state to thread through
// `AppState` beyond the handle returned here, which is only needed to
// render the scrape response on `GET /metrics`.
pub fn install() -> PrometheusHandle {
    PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder")
}
