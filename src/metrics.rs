use anyhow::Result;
use opentelemetry::{
    global,
    metrics::{Counter, Meter},
    KeyValue,
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use std::sync::Arc;

pub struct MetricsCollector {
    deleted_books_counter: Counter<u64>,
    kindle_id_fetched_counter: Counter<u64>,
    price_fetched_counter: Counter<u64>,
}

impl MetricsCollector {
    pub fn new() -> Result<Arc<Self>> {
        let meter = Self::init_meter()?;

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

        Ok(Arc::new(Self {
            deleted_books_counter,
            kindle_id_fetched_counter,
            price_fetched_counter,
        }))
    }

    fn init_meter() -> Result<Meter> {
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
