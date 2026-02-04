use opentelemetry::{
    global,
    metrics::{Counter, Meter},
    KeyValue,
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use std::sync::Arc;
use tracing::{info, warn};

pub struct MetricsCollector {
    deleted_books_counter: Counter<u64>,
    kindle_id_fetched_counter: Counter<u64>,
    price_fetched_counter: Counter<u64>,
}

impl MetricsCollector {
    pub fn new() -> Arc<Self> {
        let meter = Self::init_meter();

        let deleted_books_counter = meter
            .u64_counter("bookmeter.deleted_books")
            .with_description("Number of books deleted from BookMeter")
            .build();

        let kindle_id_fetched_counter = meter
            .u64_counter("bookmeter.kindle_id_fetched")
            .with_description("Number of Kindle IDs fetched")
            .build();

        let price_fetched_counter = meter
            .u64_counter("bookmeter.price_fetched")
            .with_description("Number of prices fetched for books with Kindle ID")
            .build();

        Arc::new(Self {
            deleted_books_counter,
            kindle_id_fetched_counter,
            price_fetched_counter,
        })
    }

    fn init_meter() -> Meter {
        // OTLP エンドポイントが設定されていない場合は Noop メーターを返す
        if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err() {
            info!("OTEL_EXPORTER_OTLP_ENDPOINT not set, using noop metrics");
            return global::meter("bookmeter-discounts");
        }

        // OTLP 接続を試みる
        match Self::try_init_otlp_meter() {
            Ok(meter) => {
                info!("OpenTelemetry metrics initialized successfully");
                meter
            }
            Err(e) => {
                warn!(
                    "Failed to initialize OpenTelemetry metrics: {:?}, using noop metrics",
                    e
                );
                global::meter("bookmeter-discounts")
            }
        }
    }

    fn try_init_otlp_meter() -> Result<Meter, Box<dyn std::error::Error>> {
        // Build metrics exporter using the public API
        let exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_timeout(std::time::Duration::from_secs(10))
            .build()?;

        let reader = PeriodicReader::builder(exporter)
            .with_interval(std::time::Duration::from_secs(60))
            .build();

        let provider = SdkMeterProvider::builder().with_reader(reader).build();

        global::set_meter_provider(provider);

        Ok(global::meter("bookmeter-discounts"))
    }

    pub fn record_deleted_book(&self) {
        self.deleted_books_counter
            .add(1, &[KeyValue::new("operation", "delete_from_bookmeter")]);
    }

    pub fn record_kindle_id_fetched(&self) {
        self.kindle_id_fetched_counter
            .add(1, &[KeyValue::new("operation", "fetch_kindle_id")]);
    }

    pub fn record_price_fetched(&self) {
        self.price_fetched_counter
            .add(1, &[KeyValue::new("operation", "fetch_price")]);
    }
}
