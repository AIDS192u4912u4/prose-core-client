#[allow(non_snake_case)]
pub fn enableLogging() {
    println!("4");
    use log::{LevelFilter, Metadata, Record, SetLoggerError};

    pub struct Logger;

    impl log::Log for Logger {
        fn enabled(&self, _metadata: &Metadata) -> bool {
            true
        }

        fn log(&self, record: &Record) {
            if self.enabled(record.metadata()) {
                println!("({}) - {}", record.level(), record.args());
            }
        }

        fn flush(&self) {}
    }

    impl Logger {
        pub fn init(level: LevelFilter) -> Result<(), SetLoggerError> {
            log::set_max_level(level);
            log::set_boxed_logger(Box::new(Logger))
        }
    }

    _ = Logger::init(LevelFilter::Trace);
}
